// Copyright 2018 Amazon.com, Inc. or its affiliates. All Rights Reserved.
//
// Portions Copyright 2017 The Chromium OS Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE-BSD-3-Clause file.
//
// Copyright © 2019 Intel Corporation
//
// SPDX-License-Identifier: Apache-2.0 AND BSD-3-Clause

use crate::VirtioIommuRemapping;
use std::cmp::min;
use std::convert::TryInto;
use std::fmt::{self, Display};
use std::mem::size_of;
use std::num::Wrapping;
use std::sync::atomic::{fence, Ordering};
use std::sync::Arc;
use vm_memory::{
    Address, ByteValued, Bytes, GuestAddress, GuestMemory, GuestMemoryError, GuestMemoryMmap,
    GuestUsize, VolatileMemory,
};

pub const VIRTQ_DESC_F_NEXT: u16 = 0x1;
pub const VIRTQ_DESC_F_WRITE: u16 = 0x2;
pub const VIRTQ_DESC_F_INDIRECT: u16 = 0x4;

#[derive(Debug)]
pub enum Error {
    GuestMemoryError,
    InvalidIndirectDescriptor,
    InvalidChain,
    InvalidOffset(u64),
    InvalidRingIndexFromMemory(GuestMemoryError),
}

impl Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use self::Error::*;

        match self {
            GuestMemoryError => write!(f, "error accessing guest memory"),
            InvalidChain => write!(f, "invalid descriptor chain"),
            InvalidIndirectDescriptor => write!(f, "invalid indirect descriptor"),
            InvalidOffset(o) => write!(f, "invalid offset {}", o),
            InvalidRingIndexFromMemory(e) => write!(f, "invalid ring index from memory: {}", e),
        }
    }
}

/// A virtio descriptor constraints with C representation
#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct Descriptor {
    /// Guest physical address of device specific data
    addr: u64,

    /// Length of device specific data
    len: u32,

    /// Includes next, write, and indirect bits
    flags: u16,

    /// Index into the descriptor table of the next descriptor if flags has
    /// the next bit set
    next: u16,
}

// GuestMemoryMmap::read_obj() will be used to fetch the descriptor,
// which has an explicit constraint that the entire descriptor doesn't
// cross the page boundary. Otherwise the descriptor may be splitted into
// two mmap regions which causes failure of GuestMemoryMmap::read_obj().
//
// The Virtio Spec 1.0 defines the alignment of VirtIO descriptor is 16 bytes,
// which fulfills the explicit constraint of GuestMemoryMmap::read_obj().
impl Descriptor {
    /// Return the guest physical address of descriptor buffer
    pub fn addr(&self) -> GuestAddress {
        GuestAddress(self.addr)
    }

    /// Return the length of descriptor buffer
    pub fn len(&self) -> u32 {
        self.len
    }

    /// Check if this is an empty descriptor.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Return the flags for this descriptor, including next, write and indirect
    /// bits
    pub fn flags(&self) -> u16 {
        self.flags
    }

    /// Checks if the driver designated this as a write only descriptor.
    ///
    /// If this is false, this descriptor is read only.
    /// Write only means the the emulated device can write and the driver can read.
    pub fn is_write_only(&self) -> bool {
        self.flags & VIRTQ_DESC_F_WRITE != 0
    }

    /// Checks if this descriptor has another descriptor linked after it.
    pub fn has_next(&self) -> bool {
        self.flags & VIRTQ_DESC_F_NEXT != 0
    }
}

unsafe impl ByteValued for Descriptor {}

/// A virtio descriptor head, not tied to a GuestMemoryMmap.
pub struct DescriptorHead {
    desc_table: GuestAddress,
    table_size: u16,
    index: u16,
    iommu_mapping_cb: Option<Arc<VirtioIommuRemapping>>,
}

/// A virtio descriptor chain.
#[derive(Clone)]
pub struct DescriptorChain<'a> {
    desc_table: GuestAddress,
    table_size: u16,
    ttl: u16,   // used to prevent infinite chain cycles
    index: u16, // Index into the descriptor table
    iommu_mapping_cb: Option<Arc<VirtioIommuRemapping>>,

    /// Reference to guest memory
    pub mem: &'a GuestMemoryMmap,

    /// This particular descriptor
    pub desc: Descriptor,
}

impl<'a> DescriptorChain<'a> {
    pub fn read_new(
        mem: &'a GuestMemoryMmap,
        desc_table: GuestAddress,
        table_size: u16,
        ttl: u16,
        index: u16,
        iommu_mapping_cb: Option<Arc<VirtioIommuRemapping>>,
    ) -> Option<Self> {
        if index >= table_size {
            return None;
        }

        let desc_table_size = size_of::<Descriptor>() * table_size as usize;
        let slice = mem.get_slice(desc_table, desc_table_size).ok()?;
        let mut desc = slice
            .get_array_ref::<Descriptor>(0, table_size as usize)
            .ok()?
            .load(index as usize);

        // Translate address if necessary
        if let Some(iommu_mapping_cb) = &iommu_mapping_cb {
            desc.addr = (iommu_mapping_cb)(desc.addr).unwrap()
        }

        let chain = DescriptorChain {
            mem,
            desc_table,
            table_size,
            ttl,
            index,
            desc,
            iommu_mapping_cb,
        };

        if chain.is_valid() {
            Some(chain)
        } else {
            None
        }
    }

    pub fn checked_new(
        mem: &'a GuestMemoryMmap,
        dtable_addr: GuestAddress,
        table_size: u16,
        index: u16,
        iommu_mapping_cb: Option<Arc<VirtioIommuRemapping>>,
    ) -> Option<Self> {
        Self::read_new(
            mem,
            dtable_addr,
            table_size,
            table_size,
            index,
            iommu_mapping_cb,
        )
    }

    pub fn new_from_indirect(&self) -> Result<DescriptorChain, Error> {
        if !self.is_indirect() {
            return Err(Error::InvalidIndirectDescriptor);
        }

        let desc_head = self.desc.addr();
        self.mem
            .checked_offset(desc_head, 16)
            .ok_or(Error::GuestMemoryError)?;

        // These reads can't fail unless Guest memory is hopelessly broken.
        let mut desc = match self.mem.read_obj::<Descriptor>(desc_head) {
            Ok(ret) => ret,
            Err(_) => return Err(Error::GuestMemoryError),
        };

        // Translate address if necessary
        let iommu_mapping_cb = if let Some(iommu_mapping_cb) = self.iommu_mapping_cb.clone() {
            desc.addr = (iommu_mapping_cb)(desc.addr).unwrap();
            Some(iommu_mapping_cb)
        } else {
            None
        };

        let chain = DescriptorChain {
            mem: self.mem,
            desc_table: self.desc.addr(),
            table_size: (self.desc.len() / 16).try_into().unwrap(),
            ttl: (self.desc.len() / 16).try_into().unwrap(),
            index: 0,
            desc,
            iommu_mapping_cb,
        };

        if !chain.is_valid() {
            return Err(Error::InvalidChain);
        }

        Ok(chain)
    }

    /// Returns a copy of a descriptor referencing a different GuestMemoryMmap object.
    pub fn new_from_head(
        mem: &'a GuestMemoryMmap,
        head: DescriptorHead,
    ) -> Result<DescriptorChain<'a>, Error> {
        match DescriptorChain::checked_new(
            mem,
            head.desc_table,
            head.table_size,
            head.index,
            head.iommu_mapping_cb,
        ) {
            Some(d) => Ok(d),
            None => Err(Error::InvalidChain),
        }
    }

    /// Returns a DescriptorHead that can be used to build a copy of a descriptor
    /// referencing a different GuestMemoryMmap.
    pub fn get_head(&self) -> DescriptorHead {
        DescriptorHead {
            desc_table: self.desc_table,
            table_size: self.table_size,
            index: self.index,
            iommu_mapping_cb: self.iommu_mapping_cb.clone(),
        }
    }

    fn is_valid(&self) -> bool {
        !(self
            .mem
            .checked_offset(self.desc.addr(), self.desc.len as usize)
            .is_none()
            || (self.has_next() && self.desc.next >= self.table_size))
    }

    /// Gets if this descriptor chain has another descriptor chain linked after it.
    pub fn has_next(&self) -> bool {
        self.desc.flags & VIRTQ_DESC_F_NEXT != 0 && self.ttl > 1
    }

    /// If the driver designated this as a write only descriptor.
    ///
    /// If this is false, this descriptor is read only.
    /// Write only means the the emulated device can write and the driver can read.
    pub fn is_write_only(&self) -> bool {
        self.desc.flags & VIRTQ_DESC_F_WRITE != 0
    }

    pub fn is_indirect(&self) -> bool {
        self.desc.flags & VIRTQ_DESC_F_INDIRECT != 0
    }

    /// Get the descriptor index of the chain header
    pub fn index(&self) -> u16 {
        self.index
    }

    /// Return the guest physical address of descriptor buffer
    pub fn addr(&self) -> GuestAddress {
        GuestAddress(self.desc.addr)
    }

    /// Return the length of descriptor buffer
    pub fn len(&self) -> u32 {
        self.desc.len
    }

    /// Return the flags for this descriptor, including next, write and indirect
    /// bits
    pub fn flags(&self) -> u16 {
        self.desc.flags
    }

    /// Check if this is an empty descriptor.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns an iterator that only yields the readable descriptors in the chain.
    pub fn readable(self) -> DescriptorChainRwIter<'a> {
        DescriptorChainRwIter {
            chain: self,
            writable: false,
        }
    }

    /// Returns an iterator that only yields the writable descriptors in the chain.
    pub fn writable(self) -> DescriptorChainRwIter<'a> {
        DescriptorChainRwIter {
            chain: self,
            writable: true,
        }
    }
}

impl<'a> Iterator for DescriptorChain<'a> {
    type Item = Descriptor;

    /// Returns the next descriptor in this descriptor chain, if there is one.
    ///
    /// Note that this is distinct from the next descriptor chain returned by
    /// [`AvailIter`](struct.AvailIter.html), which is the head of the next
    /// _available_ descriptor chain.
    fn next(&mut self) -> Option<Self::Item> {
        if self.ttl == 0 {
            return None;
        }

        let curr = self.desc;
        if !self.has_next() {
            self.ttl = 0
        } else {
            let index = self.desc.next;
            let desc_table_size = size_of::<Descriptor>() * self.table_size as usize;
            let slice = self.mem.get_slice(self.desc_table, desc_table_size).ok()?;
            self.desc = slice
                .get_array_ref(0, self.table_size as usize)
                .ok()?
                .load(index as usize);
            self.ttl -= 1;
        }
        Some(curr)
    }
}

/// An iterator for readable or writable descriptors.
pub struct DescriptorChainRwIter<'a> {
    chain: DescriptorChain<'a>,
    writable: bool,
}

impl<'a> Iterator for DescriptorChainRwIter<'a> {
    type Item = Descriptor;

    /// Returns the next descriptor in this descriptor chain, if there is one.
    ///
    /// Note that this is distinct from the next descriptor chain returned by
    /// [`AvailIter`](struct.AvailIter.html), which is the head of the next
    /// _available_ descriptor chain.
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.chain.next() {
                Some(v) => {
                    if v.is_write_only() == self.writable {
                        return Some(v);
                    }
                }
                None => return None,
            }
        }
    }
}

/// Consuming iterator over all available descriptor chain heads in the queue.
pub struct AvailIter<'a, 'b> {
    mem: &'a GuestMemoryMmap,
    desc_table: GuestAddress,
    avail_ring: GuestAddress,
    next_index: Wrapping<u16>,
    last_index: Wrapping<u16>,
    queue_size: u16,
    next_avail: &'b mut Wrapping<u16>,
    iommu_mapping_cb: Option<Arc<VirtioIommuRemapping>>,
}

impl<'a, 'b> AvailIter<'a, 'b> {
    pub fn new(mem: &'a GuestMemoryMmap, q_next_avail: &'b mut Wrapping<u16>) -> AvailIter<'a, 'b> {
        AvailIter {
            mem,
            desc_table: GuestAddress(0),
            avail_ring: GuestAddress(0),
            next_index: Wrapping(0),
            last_index: Wrapping(0),
            queue_size: 0,
            next_avail: q_next_avail,
            iommu_mapping_cb: None,
        }
    }
}

impl<'a, 'b> Iterator for AvailIter<'a, 'b> {
    type Item = DescriptorChain<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.next_index == self.last_index {
            return None;
        }

        let offset = (4 + (self.next_index.0 % self.queue_size) * 2) as usize;
        let avail_addr = match self.mem.checked_offset(self.avail_ring, offset) {
            Some(a) => a,
            None => return None,
        };
        // This index is checked below in checked_new
        let desc_index: u16 = match self.mem.read_obj(avail_addr) {
            Ok(ret) => ret,
            Err(_) => {
                // TODO log address
                error!("Failed to read from memory");
                return None;
            }
        };

        self.next_index += Wrapping(1);

        let ret = DescriptorChain::checked_new(
            self.mem,
            self.desc_table,
            self.queue_size,
            desc_index,
            self.iommu_mapping_cb.clone(),
        );
        if ret.is_some() {
            *self.next_avail += Wrapping(1);
        }
        ret
    }
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "GuestAddress")]
struct GuestAddressDef(pub u64);

#[derive(Clone, Serialize, Deserialize)]
/// A virtio queue's parameters.
pub struct Queue {
    /// The maximal size in elements offered by the device
    pub max_size: u16,

    /// The queue size in elements the driver selected
    pub size: u16,

    /// Indicates if the queue is finished with configuration
    pub ready: bool,

    /// Interrupt vector index of the queue
    pub vector: u16,

    #[serde(with = "GuestAddressDef")]
    /// Guest physical address of the descriptor table
    pub desc_table: GuestAddress,

    #[serde(with = "GuestAddressDef")]
    /// Guest physical address of the available ring
    pub avail_ring: GuestAddress,

    #[serde(with = "GuestAddressDef")]
    /// Guest physical address of the used ring
    pub used_ring: GuestAddress,

    pub next_avail: Wrapping<u16>,
    pub next_used: Wrapping<u16>,

    #[serde(skip)]
    pub iommu_mapping_cb: Option<Arc<VirtioIommuRemapping>>,

    /// VIRTIO_F_RING_EVENT_IDX negotiated
    event_idx: bool,

    /// The last used value when using EVENT_IDX
    signalled_used: Option<Wrapping<u16>>,
}

impl Queue {
    /// Constructs an empty virtio queue with the given `max_size`.
    pub fn new(max_size: u16) -> Queue {
        Queue {
            max_size,
            size: max_size,
            ready: false,
            vector: 0,
            desc_table: GuestAddress(0),
            avail_ring: GuestAddress(0),
            used_ring: GuestAddress(0),
            next_avail: Wrapping(0),
            next_used: Wrapping(0),
            iommu_mapping_cb: None,
            event_idx: false,
            signalled_used: None,
        }
    }

    pub fn get_max_size(&self) -> u16 {
        self.max_size
    }

    pub fn enable(&mut self, set: bool) {
        self.ready = set;

        if set {
            // Translate address of descriptor table and vrings.
            if let Some(iommu_mapping_cb) = &self.iommu_mapping_cb {
                self.desc_table =
                    GuestAddress((iommu_mapping_cb)(self.desc_table.raw_value()).unwrap());
                self.avail_ring =
                    GuestAddress((iommu_mapping_cb)(self.avail_ring.raw_value()).unwrap());
                self.used_ring =
                    GuestAddress((iommu_mapping_cb)(self.used_ring.raw_value()).unwrap());
            }
        } else {
            self.desc_table = GuestAddress(0);
            self.avail_ring = GuestAddress(0);
            self.used_ring = GuestAddress(0);
        }
    }

    /// Return the actual size of the queue, as the driver may not set up a
    /// queue as big as the device allows.
    pub fn actual_size(&self) -> u16 {
        min(self.size, self.max_size)
    }

    /// Reset the queue to a state that is acceptable for a device reset
    pub fn reset(&mut self) {
        self.ready = false;
        self.size = self.max_size;
        self.next_avail = Wrapping(0);
        self.next_used = Wrapping(0);
    }

    pub fn is_valid(&self, mem: &GuestMemoryMmap) -> bool {
        let queue_size = self.actual_size() as usize;
        let desc_table = self.desc_table;
        let desc_table_size = 16 * queue_size;
        let avail_ring = self.avail_ring;
        let avail_ring_size = 6 + 2 * queue_size;
        let used_ring = self.used_ring;
        let used_ring_size = 6 + 8 * queue_size;
        if !self.ready {
            error!("attempt to use virtio queue that is not marked ready");
            false
        } else if self.size > self.max_size || self.size == 0 || (self.size & (self.size - 1)) != 0
        {
            error!("virtio queue with invalid size: {}", self.size);
            false
        } else if desc_table
            .checked_add(desc_table_size as GuestUsize)
            .map_or(true, |v| !mem.address_in_range(v))
        {
            error!(
                "virtio queue descriptor table goes out of bounds: start:0x{:08x} size:0x{:08x}",
                desc_table.raw_value(),
                desc_table_size
            );
            false
        } else if avail_ring
            .checked_add(avail_ring_size as GuestUsize)
            .map_or(true, |v| !mem.address_in_range(v))
        {
            error!(
                "virtio queue available ring goes out of bounds: start:0x{:08x} size:0x{:08x}",
                avail_ring.raw_value(),
                avail_ring_size
            );
            false
        } else if used_ring
            .checked_add(used_ring_size as GuestUsize)
            .map_or(true, |v| !mem.address_in_range(v))
        {
            error!(
                "virtio queue used ring goes out of bounds: start:0x{:08x} size:0x{:08x}",
                used_ring.raw_value(),
                used_ring_size
            );
            false
        } else if desc_table.mask(0xf) != 0 {
            error!("virtio queue descriptor table breaks alignment contraints");
            false
        } else if avail_ring.mask(0x1) != 0 {
            error!("virtio queue available ring breaks alignment contraints");
            false
        } else if used_ring.mask(0x3) != 0 {
            error!("virtio queue used ring breaks alignment contraints");
            false
        } else {
            true
        }
    }

    /// A consuming iterator over all available descriptor chain heads offered by the driver.
    pub fn iter<'a, 'b>(&'b mut self, mem: &'a GuestMemoryMmap) -> AvailIter<'a, 'b> {
        let queue_size = self.actual_size();
        let avail_ring = self.avail_ring;

        let index_addr = match mem.checked_offset(avail_ring, 2) {
            Some(ret) => ret,
            None => {
                // TODO log address
                warn!("Invalid offset");
                return AvailIter::new(mem, &mut self.next_avail);
            }
        };
        // Note that last_index has no invalid values
        let last_index: u16 = match mem.read_obj::<u16>(index_addr) {
            Ok(ret) => ret,
            Err(_) => return AvailIter::new(mem, &mut self.next_avail),
        };

        AvailIter {
            mem,
            desc_table: self.desc_table,
            avail_ring,
            next_index: self.next_avail,
            last_index: Wrapping(last_index),
            queue_size,
            next_avail: &mut self.next_avail,
            iommu_mapping_cb: self.iommu_mapping_cb.clone(),
        }
    }

    /// Update avail_event on the used ring with the last index in the avail ring.
    pub fn update_avail_event(&mut self, mem: &GuestMemoryMmap) {
        let index_addr = match mem.checked_offset(self.avail_ring, 2) {
            Some(ret) => ret,
            None => {
                // TODO log address
                warn!("Invalid offset");
                return;
            }
        };
        // Note that last_index has no invalid values
        let last_index: u16 = match mem.read_obj::<u16>(index_addr) {
            Ok(ret) => ret,
            Err(_) => return,
        };

        match mem.checked_offset(self.used_ring, (4 + self.actual_size() * 8) as usize) {
            Some(a) => {
                mem.write_obj(last_index, a).unwrap();
            }
            None => warn!("Can't update avail_event"),
        }

        // This fence ensures the guest sees the value we've just written.
        fence(Ordering::Release);
    }

    /// Return the value present in the used_event field of the avail ring.
    #[inline(always)]
    pub fn get_used_event(&self, mem: &GuestMemoryMmap) -> Option<Wrapping<u16>> {
        let avail_ring = self.avail_ring;
        let used_event_addr =
            match mem.checked_offset(avail_ring, (4 + self.actual_size() * 2) as usize) {
                Some(a) => a,
                None => {
                    warn!("Invalid offset looking for used_event");
                    return None;
                }
            };

        // This fence ensures we're seeing the latest update from the guest.
        fence(Ordering::SeqCst);
        match mem.read_obj::<u16>(used_event_addr) {
            Ok(ret) => Some(Wrapping(ret)),
            Err(_) => None,
        }
    }

    /// Puts an available descriptor head into the used ring for use by the guest.
    pub fn add_used(&mut self, mem: &GuestMemoryMmap, desc_index: u16, len: u32) -> Option<u16> {
        if desc_index >= self.actual_size() {
            error!(
                "attempted to add out of bounds descriptor to used ring: {}",
                desc_index
            );
            return None;
        }

        let used_ring = self.used_ring;
        let next_used = u64::from(self.next_used.0 % self.actual_size());
        let used_elem = used_ring.unchecked_add(4 + next_used * 8);

        // These writes can't fail as we are guaranteed to be within the descriptor ring.
        mem.write_obj(u32::from(desc_index), used_elem).unwrap();
        mem.write_obj(len as u32, used_elem.unchecked_add(4))
            .unwrap();

        self.next_used += Wrapping(1);

        // This fence ensures all descriptor writes are visible before the index update is.
        fence(Ordering::Release);

        mem.write_obj(self.next_used.0 as u16, used_ring.unchecked_add(2))
            .unwrap();

        Some(self.next_used.0)
    }

    /// Goes back one position in the available descriptor chain offered by the driver.
    /// Rust does not support bidirectional iterators. This is the only way to revert the effect
    /// of an iterator increment on the queue.
    pub fn go_to_previous_position(&mut self) {
        self.next_avail -= Wrapping(1);
    }

    /// Get ring's index from memory.
    fn index_from_memory(&self, ring: GuestAddress, mem: &GuestMemoryMmap) -> Result<u16, Error> {
        mem.read_obj::<u16>(
            mem.checked_offset(ring, 2)
                .ok_or_else(|| Error::InvalidOffset(ring.raw_value() + 2))?,
        )
        .map_err(Error::InvalidRingIndexFromMemory)
    }

    /// Get latest index from available ring.
    pub fn avail_index_from_memory(&self, mem: &GuestMemoryMmap) -> Result<u16, Error> {
        self.index_from_memory(self.avail_ring, mem)
    }

    /// Get latest index from used ring.
    pub fn used_index_from_memory(&self, mem: &GuestMemoryMmap) -> Result<u16, Error> {
        self.index_from_memory(self.used_ring, mem)
    }

    pub fn available_descriptors(&self, mem: &GuestMemoryMmap) -> Result<bool, Error> {
        Ok(self.used_index_from_memory(mem)? < self.avail_index_from_memory(mem)?)
    }

    pub fn set_event_idx(&mut self, enabled: bool) {
        /* Also reset the last signalled event */
        self.signalled_used = None;
        self.event_idx = enabled;
    }

    pub fn needs_notification(&mut self, mem: &GuestMemoryMmap, used_idx: Wrapping<u16>) -> bool {
        if !self.event_idx {
            return true;
        }

        let mut notify = true;

        if let Some(old_idx) = self.signalled_used {
            if let Some(used_event) = self.get_used_event(&mem) {
                info!(
                    "used_event = {:?} used_idx = {:?} old_idx = {:?}",
                    used_event, used_idx, old_idx
                );
                if (used_idx - used_event - Wrapping(1u16)) >= (used_idx - old_idx) {
                    notify = false;
                }
            }
        }

        self.signalled_used = Some(used_idx);
        info!("Needs notification: {:?}", notify);
        notify
    }
}

#[macro_use]
pub mod testing {
    extern crate vm_memory;

    pub use super::*;
    use std::marker::PhantomData;
    use std::mem;
    use vm_memory::{
        GuestAddress, GuestMemoryMmap, GuestMemoryRegion, VolatileMemory, VolatileRef,
        VolatileSlice,
    };

    // Represents a virtio descriptor in guest memory.
    pub struct VirtqDesc<'a> {
        desc: VolatileSlice<'a>,
    }

    #[macro_export]
    macro_rules! offset_of {
        ($ty:ty, $field:ident) => {
            unsafe { &(*(0 as *const $ty)).$field as *const _ as usize }
        };
    }

    #[allow(clippy::len_without_is_empty)]
    #[allow(clippy::zero_ptr)]
    impl<'a> VirtqDesc<'a> {
        fn new(dtable: &'a VolatileSlice<'a>, i: u16) -> Self {
            let desc = dtable
                .get_slice((i as usize) * Self::dtable_len(1), Self::dtable_len(1))
                .unwrap();
            VirtqDesc { desc }
        }

        pub fn addr(&self) -> VolatileRef<u64> {
            self.desc.get_ref(offset_of!(Descriptor, addr)).unwrap()
        }

        pub fn len(&self) -> VolatileRef<u32> {
            self.desc.get_ref(offset_of!(Descriptor, len)).unwrap()
        }

        pub fn flags(&self) -> VolatileRef<u16> {
            self.desc.get_ref(offset_of!(Descriptor, flags)).unwrap()
        }

        pub fn next(&self) -> VolatileRef<u16> {
            self.desc.get_ref(offset_of!(Descriptor, next)).unwrap()
        }

        pub fn set(&self, addr: u64, len: u32, flags: u16, next: u16) {
            self.addr().store(addr);
            self.len().store(len);
            self.flags().store(flags);
            self.next().store(next);
        }

        fn dtable_len(nelem: u16) -> usize {
            16 * nelem as usize
        }
    }

    // Represents a virtio queue ring. The only difference between the used and available rings,
    // is the ring element type.
    pub struct VirtqRing<'a, T> {
        ring: VolatileSlice<'a>,
        start: GuestAddress,
        qsize: u16,
        _marker: PhantomData<*const T>,
    }

    impl<'a, T> VirtqRing<'a, T>
    where
        T: vm_memory::ByteValued,
    {
        fn new(
            start: GuestAddress,
            mem: &'a GuestMemoryMmap,
            qsize: u16,
            alignment: GuestUsize,
        ) -> Self {
            assert_eq!(start.0 & (alignment - 1), 0);

            let (region, addr) = mem.to_region_addr(start).unwrap();
            let size = Self::ring_len(qsize);
            let ring = region.get_slice(addr, size).unwrap();

            let result = VirtqRing {
                ring,
                start,
                qsize,
                _marker: PhantomData,
            };

            result.flags().store(0);
            result.idx().store(0);
            result.event().store(0);
            result
        }

        pub fn start(&self) -> GuestAddress {
            self.start
        }

        pub fn end(&self) -> GuestAddress {
            self.start.unchecked_add(self.ring.len() as GuestUsize)
        }

        pub fn flags(&self) -> VolatileRef<u16> {
            self.ring.get_ref(0).unwrap()
        }

        pub fn idx(&self) -> VolatileRef<u16> {
            self.ring.get_ref(2).unwrap()
        }

        fn ring_offset(i: u16) -> usize {
            4 + mem::size_of::<T>() * (i as usize)
        }

        pub fn ring(&self, i: u16) -> VolatileRef<T> {
            assert!(i < self.qsize);
            self.ring.get_ref(Self::ring_offset(i)).unwrap()
        }

        pub fn event(&self) -> VolatileRef<u16> {
            self.ring.get_ref(Self::ring_offset(self.qsize)).unwrap()
        }

        fn ring_len(qsize: u16) -> usize {
            Self::ring_offset(qsize) + 2
        }
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub struct VirtqUsedElem {
        pub id: u32,
        pub len: u32,
    }

    unsafe impl vm_memory::ByteValued for VirtqUsedElem {}

    pub type VirtqAvail<'a> = VirtqRing<'a, u16>;
    pub type VirtqUsed<'a> = VirtqRing<'a, VirtqUsedElem>;

    trait GuestAddressExt {
        fn align_up(&self, x: GuestUsize) -> GuestAddress;
    }
    impl GuestAddressExt for GuestAddress {
        fn align_up(&self, x: GuestUsize) -> GuestAddress {
            Self((self.0 + (x - 1)) & !(x - 1))
        }
    }

    pub struct VirtQueue<'a> {
        start: GuestAddress,
        dtable: VolatileSlice<'a>,
        avail: VirtqAvail<'a>,
        used: VirtqUsed<'a>,
    }

    impl<'a> VirtQueue<'a> {
        // We try to make sure things are aligned properly :-s
        pub fn new(start: GuestAddress, mem: &'a GuestMemoryMmap, qsize: u16) -> Self {
            // power of 2?
            assert!(qsize > 0 && qsize & (qsize - 1) == 0);

            let (region, addr) = mem.to_region_addr(start).unwrap();
            let dtable = region
                .get_slice(addr, VirtqDesc::dtable_len(qsize))
                .unwrap();

            const AVAIL_ALIGN: u64 = 2;

            let avail_addr = start
                .unchecked_add(VirtqDesc::dtable_len(qsize) as GuestUsize)
                .align_up(AVAIL_ALIGN);
            let avail = VirtqAvail::new(avail_addr, mem, qsize, AVAIL_ALIGN);

            const USED_ALIGN: u64 = 4;

            let used_addr = avail.end().align_up(USED_ALIGN);
            let used = VirtqUsed::new(used_addr, mem, qsize, USED_ALIGN);

            VirtQueue {
                start,
                dtable,
                avail,
                used,
            }
        }

        fn size(&self) -> u16 {
            (self.dtable.len() / VirtqDesc::dtable_len(1)) as u16
        }

        pub fn dtable(&self, i: u16) -> VirtqDesc {
            VirtqDesc::new(&self.dtable, i)
        }

        pub fn avail(&self) -> &VirtqAvail {
            &self.avail
        }

        pub fn used(&self) -> &VirtqUsed {
            &self.used
        }

        pub fn dtable_start(&self) -> GuestAddress {
            self.start
        }

        pub fn avail_start(&self) -> GuestAddress {
            self.avail.start()
        }

        pub fn used_start(&self) -> GuestAddress {
            self.used.start()
        }

        // Creates a new Queue, using the underlying memory regions represented by the VirtQueue.
        pub fn create_queue(&self) -> Queue {
            let mut q = Queue::new(self.size());

            q.size = self.size();
            q.ready = true;
            q.desc_table = self.dtable_start();
            q.avail_ring = self.avail_start();
            q.used_ring = self.used_start();

            q
        }

        pub fn start(&self) -> GuestAddress {
            self.dtable_start()
        }

        pub fn end(&self) -> GuestAddress {
            self.used.end()
        }
    }
}

#[cfg(test)]
pub mod tests {
    extern crate vm_memory;

    use super::testing::*;
    pub use super::*;
    use vm_memory::{GuestAddress, GuestMemoryMmap};

    #[test]
    fn test_checked_new_descriptor_chain() {
        let m = &GuestMemoryMmap::from_ranges(&[(GuestAddress(0), 0x10000)]).unwrap();
        let vq = VirtQueue::new(GuestAddress(0), m, 16);

        assert!(vq.end().0 < 0x1000);

        // index >= queue_size
        assert!(DescriptorChain::checked_new(m, vq.start(), 16, 16, None).is_none());

        // desc_table address is way off
        assert!(
            DescriptorChain::checked_new(m, GuestAddress(0x00ff_ffff_ffff), 16, 0, None).is_none()
        );

        // the addr field of the descriptor is way off
        vq.dtable(0).addr().store(0x0fff_ffff_ffff);
        assert!(DescriptorChain::checked_new(m, vq.start(), 16, 0, None).is_none());

        // let's create some invalid chains

        {
            // the addr field of the desc is ok now
            vq.dtable(0).addr().store(0x1000);
            // ...but the length is too large
            vq.dtable(0).len().store(0xffff_ffff);
            assert!(DescriptorChain::checked_new(m, vq.start(), 16, 0, None).is_none());
        }

        {
            // the first desc has a normal len now, and the next_descriptor flag is set
            vq.dtable(0).len().store(0x1000);
            vq.dtable(0).flags().store(VIRTQ_DESC_F_NEXT);
            //..but the the index of the next descriptor is too large
            vq.dtable(0).next().store(16);

            assert!(DescriptorChain::checked_new(m, vq.start(), 16, 0, None).is_none());
        }

        // finally, let's test an ok chain

        {
            vq.dtable(0).next().store(1);
            vq.dtable(1).set(0x2000, 0x1000, 0, 0);

            let mut c = DescriptorChain::checked_new(m, vq.start(), 16, 0, None).unwrap();

            assert_eq!(c.mem as *const GuestMemoryMmap, m as *const GuestMemoryMmap);
            assert_eq!(c.desc_table, vq.start());
            assert_eq!(c.table_size, 16);
            assert_eq!(c.ttl, c.table_size);
            assert_eq!(c.index(), 0);
            let desc = c.next().unwrap();
            assert_eq!(desc.addr(), GuestAddress(0x1000));
            assert_eq!(desc.len(), 0x1000);
            assert_eq!(desc.flags(), VIRTQ_DESC_F_NEXT);
            assert_eq!(desc.next, 1);

            assert!(c.next().is_some());
            assert!(c.next().is_none());
        }
    }

    #[test]
    fn test_new_from_descriptor_chain() {
        let m = &GuestMemoryMmap::from_ranges(&[(GuestAddress(0), 0x10000)]).unwrap();
        let vq = VirtQueue::new(GuestAddress(0), m, 16);

        // create a chain with a descriptor pointing to an indirect table
        vq.dtable(0).addr().store(0x1000);
        vq.dtable(0).len().store(0x1000);
        vq.dtable(0).next().store(0);
        vq.dtable(0).flags().store(VIRTQ_DESC_F_INDIRECT);

        let desc_chain = DescriptorChain::checked_new(m, vq.start(), 16, 0, None).unwrap();
        assert!(desc_chain.is_indirect());

        // Create the indirect chain, at 0x1000.
        let vq_indirect = VirtQueue::new(GuestAddress(0x1000), m, 16);
        for j in 0..4 {
            vq_indirect
                .dtable(j)
                .set(0x1000, 0x1000, VIRTQ_DESC_F_NEXT, (j + 1) as u16);
        }

        // try to iterate through the indirect table descriptors
        let mut indirect_desc_chain = desc_chain.new_from_indirect().unwrap();
        let mut indirect_desc = indirect_desc_chain.next().unwrap();
        for j in 0..4 {
            assert_eq!(indirect_desc.flags, VIRTQ_DESC_F_NEXT);
            assert_eq!(indirect_desc.next, j + 1);
            indirect_desc = indirect_desc_chain.next().unwrap();
        }
    }

    #[test]
    fn test_queue_and_iterator() {
        let m = &GuestMemoryMmap::from_ranges(&[(GuestAddress(0), 0x10000)]).unwrap();
        let vq = VirtQueue::new(GuestAddress(0), m, 16);

        let mut q = vq.create_queue();

        // q is currently valid
        assert!(q.is_valid(m));

        // shouldn't be valid when not marked as ready
        q.ready = false;
        assert!(!q.is_valid(m));
        q.ready = true;

        // or when size > max_size
        q.size = q.max_size << 1;
        assert!(!q.is_valid(m));
        q.size = q.max_size;

        // or when size is 0
        q.size = 0;
        assert!(!q.is_valid(m));
        q.size = q.max_size;

        // or when size is not a power of 2
        q.size = 11;
        assert!(!q.is_valid(m));
        q.size = q.max_size;

        // or if the various addresses are off

        q.desc_table = GuestAddress(0xffff_ffff);
        assert!(!q.is_valid(m));
        q.desc_table = GuestAddress(0x1001);
        assert!(!q.is_valid(m));
        q.desc_table = vq.dtable_start();

        q.avail_ring = GuestAddress(0xffff_ffff);
        assert!(!q.is_valid(m));
        q.avail_ring = GuestAddress(0x1001);
        assert!(!q.is_valid(m));
        q.avail_ring = vq.avail_start();

        q.used_ring = GuestAddress(0xffff_ffff);
        assert!(!q.is_valid(m));
        q.used_ring = GuestAddress(0x1001);
        assert!(!q.is_valid(m));
        q.used_ring = vq.used_start();

        {
            // an invalid queue should return an iterator with no next
            q.ready = false;
            let mut i = q.iter(m);
            assert!(i.next().is_none());
        }

        q.ready = true;

        // now let's create two simple descriptor chains

        {
            for j in 0..5 {
                vq.dtable(j).set(
                    0x1000 * (j + 1) as u64,
                    0x1000,
                    VIRTQ_DESC_F_NEXT,
                    (j + 1) as u16,
                );
            }

            // the chains are (0, 1) and (2, 3, 4)
            vq.dtable(1).flags().store(0);
            vq.dtable(4).flags().store(0);
            vq.avail().ring(0).store(0);
            vq.avail().ring(1).store(2);
            vq.avail().idx().store(2);

            let mut i = q.iter(m);

            {
                let mut c = i.next().unwrap();
                c.next().unwrap();
                assert!(!c.has_next());
                assert!(c.next().is_some());
                assert!(c.next().is_none());
            }

            {
                let mut c = i.next().unwrap();
                c.next().unwrap();
                c.next().unwrap();
                c.next().unwrap();
                assert!(!c.has_next());
                assert!(c.next().is_none());
            }
        }

        // also test go_to_previous_position() works as expected
        {
            assert!(q.iter(m).next().is_none());
            q.go_to_previous_position();
            let mut c = q.iter(m).next().unwrap();
            c.next().unwrap();
            c.next().unwrap();
            c.next().unwrap();
            assert!(!c.has_next());
            assert!(c.next().is_none());
        }
    }

    #[test]
    #[allow(clippy::zero_ptr)]
    pub fn test_offset() {
        assert_eq!(offset_of!(Descriptor, addr), 0);
        assert_eq!(offset_of!(Descriptor, len), 8);
        assert_eq!(offset_of!(Descriptor, flags), 12);
        assert_eq!(offset_of!(Descriptor, next), 14);
    }

    #[test]
    fn test_add_used() {
        let m = &GuestMemoryMmap::from_ranges(&[(GuestAddress(0), 0x10000)]).unwrap();
        let vq = VirtQueue::new(GuestAddress(0), m, 16);

        let mut q = vq.create_queue();
        assert_eq!(vq.used().idx().load(), 0);

        //index too large
        q.add_used(m, 16, 0x1000);
        assert_eq!(vq.used().idx().load(), 0);

        //should be ok
        q.add_used(m, 1, 0x1000);
        assert_eq!(vq.used().idx().load(), 1);
        let x = vq.used().ring(0).load();
        assert_eq!(x.id, 1);
        assert_eq!(x.len, 0x1000);
    }
}
