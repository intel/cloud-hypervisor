//
// Copyright © 2020 Intel Corporation
//
// SPDX-License-Identifier: Apache-2.0
//

#![allow(non_camel_case_types)]

//
// MOV-Move
// SDM Volume 1, Chapter 4.3
//   Copies the second operand (source operand) to the first operand (destination operand).
//

extern crate iced_x86;

use crate::arch::emulator::{EmulationError, PlatformEmulator};
use crate::arch::x86::emulator::instructions::*;
use crate::arch::x86::Exception;

macro_rules! mov_rm_r {
    ($bound:ty) => {
        fn emulate(
            &self,
            insn: &Instruction,
            state: &mut T,
            platform: &mut dyn PlatformEmulator<CpuState = T>,
        ) -> Result<(), EmulationError<Exception>> {
            let src_reg_value = get_op(&insn, 1, std::mem::size_of::<$bound>(), state, platform)
                .map_err(EmulationError::PlatformEmulationError)?;

            set_op(
                &insn,
                0,
                std::mem::size_of::<$bound>(),
                state,
                platform,
                src_reg_value,
            )
            .map_err(EmulationError::PlatformEmulationError)?;

            state.set_ip(insn.ip());

            Ok(())
        }
    };
}

macro_rules! mov_rm_imm {
    ($bound:ty) => {
        fn emulate(
            &self,
            insn: &Instruction,
            state: &mut T,
            platform: &mut dyn PlatformEmulator<CpuState = T>,
        ) -> Result<(), EmulationError<Exception>> {
            let imm = get_op(&insn, 1, std::mem::size_of::<$bound>(), state, platform)
                .map_err(EmulationError::PlatformEmulationError)?;

            set_op(
                &insn,
                0,
                std::mem::size_of::<$bound>(),
                state,
                platform,
                imm,
            )
            .map_err(EmulationError::PlatformEmulationError)?;

            state.set_ip(insn.ip());

            Ok(())
        }
    };
}

macro_rules! mov_r_rm {
    ($bound:ty) => {
        fn emulate(
            &self,
            insn: &Instruction,
            state: &mut T,
            platform: &mut dyn PlatformEmulator<CpuState = T>,
        ) -> Result<(), EmulationError<Exception>> {
            let src_value = get_op(&insn, 1, std::mem::size_of::<$bound>(), state, platform)
                .map_err(EmulationError::PlatformEmulationError)?;

            set_op(
                &insn,
                0,
                std::mem::size_of::<$bound>(),
                state,
                platform,
                src_value,
            )
            .map_err(EmulationError::PlatformEmulationError)?;

            state.set_ip(insn.ip());

            Ok(())
        }
    };
}

macro_rules! mov_r_imm {
    ($bound:ty) => {
        fn emulate(
            &self,
            insn: &Instruction,
            state: &mut T,
            platform: &mut dyn PlatformEmulator<CpuState = T>,
        ) -> Result<(), EmulationError<Exception>> {
            let imm = get_op(&insn, 1, std::mem::size_of::<$bound>(), state, platform)
                .map_err(EmulationError::PlatformEmulationError)?;

            set_op(
                &insn,
                0,
                std::mem::size_of::<$bound>(),
                state,
                platform,
                imm,
            )
            .map_err(EmulationError::PlatformEmulationError)?;

            state.set_ip(insn.ip());

            Ok(())
        }
    };
}

pub struct Mov_r8_rm8;
impl<T: CpuStateManager> InstructionHandler<T> for Mov_r8_rm8 {
    mov_r_rm!(u8);
}

pub struct Mov_r8_imm8;
impl<T: CpuStateManager> InstructionHandler<T> for Mov_r8_imm8 {
    mov_r_imm!(u8);
}

pub struct Mov_r16_rm16;
impl<T: CpuStateManager> InstructionHandler<T> for Mov_r16_rm16 {
    mov_r_rm!(u16);
}

pub struct Mov_r16_imm16;
impl<T: CpuStateManager> InstructionHandler<T> for Mov_r16_imm16 {
    mov_r_imm!(u16);
}

pub struct Mov_r32_rm32;
impl<T: CpuStateManager> InstructionHandler<T> for Mov_r32_rm32 {
    mov_r_rm!(u32);
}

pub struct Mov_r32_imm32;
impl<T: CpuStateManager> InstructionHandler<T> for Mov_r32_imm32 {
    mov_r_imm!(u32);
}

pub struct Mov_r64_rm64;
impl<T: CpuStateManager> InstructionHandler<T> for Mov_r64_rm64 {
    mov_r_rm!(u64);
}

pub struct Mov_r64_imm64;
impl<T: CpuStateManager> InstructionHandler<T> for Mov_r64_imm64 {
    mov_r_imm!(u64);
}

pub struct Mov_rm8_imm8;
impl<T: CpuStateManager> InstructionHandler<T> for Mov_rm8_imm8 {
    mov_rm_imm!(u8);
}

pub struct Mov_rm8_r8;
impl<T: CpuStateManager> InstructionHandler<T> for Mov_rm8_r8 {
    mov_rm_r!(u8);
}

pub struct Mov_rm16_imm16;
impl<T: CpuStateManager> InstructionHandler<T> for Mov_rm16_imm16 {
    mov_rm_imm!(u16);
}

pub struct Mov_rm16_r16;
impl<T: CpuStateManager> InstructionHandler<T> for Mov_rm16_r16 {
    mov_rm_r!(u16);
}

pub struct Mov_rm32_imm32;
impl<T: CpuStateManager> InstructionHandler<T> for Mov_rm32_imm32 {
    mov_rm_imm!(u32);
}

pub struct Mov_rm32_r32;
impl<T: CpuStateManager> InstructionHandler<T> for Mov_rm32_r32 {
    mov_rm_r!(u32);
}

pub struct Mov_rm64_imm32;
impl<T: CpuStateManager> InstructionHandler<T> for Mov_rm64_imm32 {
    mov_rm_imm!(u32);
}

pub struct Mov_rm64_r64;
impl<T: CpuStateManager> InstructionHandler<T> for Mov_rm64_r64 {
    mov_rm_r!(u64);
}

#[cfg(test)]
mod tests {
    #![allow(unused_mut)]
    use super::*;
    use crate::arch::x86::emulator::mock_vmm::*;

    macro_rules! hashmap {
        ($( $key: expr => $val: expr ),*) => {{
            let mut map = ::std::collections::HashMap::new();
            $( map.insert($key, $val); )*
                map
        }}
    }

    #[test]
    // mov rax,rbx
    fn test_mov_r64_r64() -> MockResult {
        let rbx: u64 = 0x8899aabbccddeeff;
        let ip: u64 = 0x1000;
        let cpu_id = 0;
        let insn = [0x48, 0x89, 0xd8];
        let mut vmm = MockVMM::new(ip, hashmap![Register::RBX => rbx], None);
        assert!(vmm.emulate_first_insn(cpu_id, &insn).is_ok());

        let rax: u64 = vmm
            .cpu_state(cpu_id)
            .unwrap()
            .read_reg(Register::RAX)
            .unwrap();
        assert_eq!(rax, rbx);

        Ok(())
    }

    #[test]
    // mov rax,0x1122334411223344
    fn test_mov_r64_imm64() -> MockResult {
        let imm64: u64 = 0x1122334411223344;
        let ip: u64 = 0x1000;
        let cpu_id = 0;
        let insn = [0x48, 0xb8, 0x44, 0x33, 0x22, 0x11, 0x44, 0x33, 0x22, 0x11];
        let mut vmm = MockVMM::new(ip, hashmap![], None);
        assert!(vmm.emulate_first_insn(cpu_id, &insn).is_ok());

        let rax: u64 = vmm
            .cpu_state(cpu_id)
            .unwrap()
            .read_reg(Register::RAX)
            .unwrap();
        assert_eq!(rax, imm64);

        Ok(())
    }

    #[test]
    // mov rax, [rax+rax]
    fn test_mov_r64_m64() -> MockResult {
        let target_rax: u64 = 0x1234567812345678;
        let mut rax: u64 = 0x100;
        let ip: u64 = 0x1000;
        let cpu_id = 0;
        let memory: [u8; 8] = target_rax.to_le_bytes();
        let insn = [0x48, 0x8b, 0x04, 0x00];
        let mut vmm = MockVMM::new(
            ip,
            hashmap![Register::RAX => rax],
            Some((rax + rax, &memory)),
        );
        assert!(vmm.emulate_first_insn(cpu_id, &insn).is_ok());

        rax = vmm
            .cpu_state(cpu_id)
            .unwrap()
            .read_reg(Register::RAX)
            .unwrap();
        assert_eq!(rax, target_rax);

        Ok(())
    }

    #[test]
    // mov al,0x11
    fn test_mov_r8_imm8() -> MockResult {
        let imm8: u8 = 0x11;
        let ip: u64 = 0x1000;
        let cpu_id = 0;
        let insn = [0xb0, 0x11];
        let mut vmm = MockVMM::new(ip, hashmap![], None);
        assert!(vmm.emulate_first_insn(cpu_id, &insn).is_ok());

        let al = vmm
            .cpu_state(cpu_id)
            .unwrap()
            .read_reg(Register::AL)
            .unwrap();
        assert_eq!(al as u8, imm8);

        Ok(())
    }

    #[test]
    // mov eax,0x11
    fn test_mov_r32_imm8() -> MockResult {
        let imm8: u8 = 0x11;
        let ip: u64 = 0x1000;
        let cpu_id = 0;
        let insn = [0xb8, 0x11, 0x00, 0x00, 0x00];
        let mut vmm = MockVMM::new(ip, hashmap![], None);
        assert!(vmm.emulate_first_insn(cpu_id, &insn).is_ok());

        let eax = vmm
            .cpu_state(cpu_id)
            .unwrap()
            .read_reg(Register::EAX)
            .unwrap();
        assert_eq!(eax as u8, imm8);

        Ok(())
    }

    #[test]
    // mov rax,0x11223344
    fn test_mov_r64_imm32() -> MockResult {
        let imm32: u32 = 0x11223344;
        let ip: u64 = 0x1000;
        let cpu_id = 0;
        let insn = [0x48, 0xc7, 0xc0, 0x44, 0x33, 0x22, 0x11];
        let mut vmm = MockVMM::new(ip, hashmap![], None);
        assert!(vmm.emulate_first_insn(cpu_id, &insn).is_ok());

        let rax: u64 = vmm
            .cpu_state(cpu_id)
            .unwrap()
            .read_reg(Register::RAX)
            .unwrap();
        assert_eq!(rax, imm32 as u64);

        Ok(())
    }

    #[test]
    // mov byte ptr [rax],dh
    fn test_mov_m8_r8() -> MockResult {
        let rax: u64 = 0x100;
        let dh: u8 = 0x99;
        let ip: u64 = 0x1000;
        let cpu_id = 0;
        let insn = [0x88, 0x30];
        let mut vmm = MockVMM::new(
            ip,
            hashmap![Register::RAX => rax, Register::DH => dh.into()],
            None,
        );
        assert!(vmm.emulate_first_insn(cpu_id, &insn).is_ok());

        let mut memory: [u8; 1] = [0; 1];
        vmm.read_memory(rax, &mut memory).unwrap();

        assert_eq!(u8::from_le_bytes(memory), dh);

        Ok(())
    }

    #[test]
    // mov dword ptr [rax],esi
    fn test_mov_m32_r32() -> MockResult {
        let rax: u64 = 0x100;
        let esi: u32 = 0x8899;
        let ip: u64 = 0x1000;
        let cpu_id = 0;
        let insn = [0x89, 0x30];
        let mut vmm = MockVMM::new(
            ip,
            hashmap![Register::RAX => rax, Register::ESI => esi.into()],
            None,
        );
        assert!(vmm.emulate_first_insn(cpu_id, &insn).is_ok());

        let mut memory: [u8; 4] = [0; 4];
        vmm.read_memory(rax, &mut memory).unwrap();

        assert_eq!(u32::from_le_bytes(memory), esi);

        Ok(())
    }

    #[test]
    // mov dword ptr [rax+0x00000001],edi
    fn test_mov_m32imm32_r32() -> MockResult {
        let rax: u64 = 0x100;
        let displacement: u64 = 0x1;
        let edi: u32 = 0x8899;
        let ip: u64 = 0x1000;
        let cpu_id = 0;
        let insn = [0x89, 0x3c, 0x05, 0x01, 0x00, 0x00, 0x00];
        let mut vmm = MockVMM::new(
            ip,
            hashmap![Register::RAX => rax, Register::EDI => edi.into()],
            None,
        );
        assert!(vmm.emulate_first_insn(cpu_id, &insn).is_ok());

        let mut memory: [u8; 4] = [0; 4];
        vmm.read_memory(rax + displacement, &mut memory).unwrap();

        assert_eq!(u32::from_le_bytes(memory), edi);

        Ok(())
    }

    #[test]
    // mov eax,dword ptr [rax+10h]
    fn test_mov_r32_m32imm32() -> MockResult {
        let rax: u64 = 0x100;
        let displacement: u64 = 0x10;
        let eax: u32 = 0xaabbccdd;
        let memory: [u8; 4] = eax.to_le_bytes();
        let ip: u64 = 0x1000;
        let cpu_id = 0;
        let insn = [0x8b, 0x40, 0x10];
        let mut vmm = MockVMM::new(
            ip,
            hashmap![Register::RAX => rax],
            Some((rax + displacement, &memory)),
        );
        assert!(vmm.emulate_first_insn(cpu_id, &insn).is_ok());

        let new_eax = vmm
            .cpu_state(cpu_id)
            .unwrap()
            .read_reg(Register::EAX)
            .unwrap();
        assert_eq!(new_eax, eax as u64);

        Ok(())
    }

    #[test]
    // mov al,byte ptr [rax+10h]
    fn test_mov_r8_m32imm32() -> MockResult {
        let rax: u64 = 0x100;
        let displacement: u64 = 0x10;
        let al: u8 = 0xaa;
        let ip: u64 = 0x1000;
        let cpu_id = 0;
        let insn = [0x8a, 0x40, 0x10];
        let memory: [u8; 1] = al.to_le_bytes();
        let mut vmm = MockVMM::new(
            ip,
            hashmap![Register::RAX => rax],
            Some((rax + displacement, &memory)),
        );
        assert!(vmm.emulate_first_insn(cpu_id, &insn).is_ok());

        let new_al = vmm
            .cpu_state(cpu_id)
            .unwrap()
            .read_reg(Register::AL)
            .unwrap();
        assert_eq!(new_al, al as u64);

        Ok(())
    }

    #[test]
    // mov rax, 0x100
    // mov rbx, qword ptr [rax+10h]
    fn test_mov_r64_imm64_and_r64_m64() -> MockResult {
        let target_rax: u64 = 0x1234567812345678;
        let rax: u64 = 0x100;
        let displacement: u64 = 0x10;
        let ip: u64 = 0x1000;
        let cpu_id = 0;
        let memory: [u8; 8] = target_rax.to_le_bytes();
        let insn = [
            0x48, 0xc7, 0xc0, 0x00, 0x01, 0x00, 0x00, // mov rax, 0x100
            0x48, 0x8b, 0x58, 0x10, // mov rbx, qword ptr [rax+10h]
        ];
        let mut vmm = MockVMM::new(ip, hashmap![], Some((rax + displacement, &memory)));
        assert!(vmm.emulate_insn(cpu_id, &insn, Some(2)).is_ok());

        let rbx: u64 = vmm
            .cpu_state(cpu_id)
            .unwrap()
            .read_reg(Register::RBX)
            .unwrap();
        assert_eq!(rbx, target_rax);

        Ok(())
    }

    #[test]
    // mov rax, 0x100
    // mov rbx, qword ptr [rax+10h]
    fn test_mov_r64_imm64_and_r64_m64_first_insn() -> MockResult {
        let target_rax: u64 = 0x1234567812345678;
        let rax: u64 = 0x100;
        let displacement: u64 = 0x10;
        let ip: u64 = 0x1000;
        let cpu_id = 0;
        let memory: [u8; 8] = target_rax.to_le_bytes();
        let insn = [
            0x48, 0xc7, 0xc0, 0x00, 0x01, 0x00, 0x00, // mov rax, 0x100
            0x48, 0x8b, 0x58, 0x10, // mov rbx, qword ptr [rax+10h]
        ];

        let mut vmm = MockVMM::new(ip, hashmap![], Some((rax + displacement, &memory)));
        // Only run the first instruction.
        assert!(vmm.emulate_first_insn(cpu_id, &insn).is_ok());

        assert_eq!(ip + 7 as u64, vmm.cpu_state(cpu_id).unwrap().ip());

        let new_rax: u64 = vmm
            .cpu_state(cpu_id)
            .unwrap()
            .read_reg(Register::RAX)
            .unwrap();
        assert_eq!(rax, new_rax);

        Ok(())
    }

    #[test]
    // mov rax, 0x100
    // mov rbx, qword ptr [rax+10h]
    // mov rax, 0x200
    fn test_mov_r64_imm64_and_r64_m64_two_insns() -> MockResult {
        let target_rax: u64 = 0x1234567812345678;
        let rax: u64 = 0x100;
        let displacement: u64 = 0x10;
        let ip: u64 = 0x1000;
        let cpu_id = 0;
        let memory: [u8; 8] = target_rax.to_le_bytes();
        let insn = [
            0x48, 0xc7, 0xc0, 0x00, 0x01, 0x00, 0x00, // mov rax, 0x100
            0x48, 0x8b, 0x58, 0x10, // mov rbx, qword ptr [rax+10h]
            0x48, 0xc7, 0xc0, 0x00, 0x02, 0x00, 0x00, // mov rax, 0x200
        ];

        let mut vmm = MockVMM::new(ip, hashmap![], Some((rax + displacement, &memory)));
        // Run the 2 first instructions.
        assert!(vmm.emulate_insn(cpu_id, &insn, Some(2)).is_ok());

        assert_eq!(ip + 7 + 4 as u64, vmm.cpu_state(cpu_id).unwrap().ip());

        let rbx: u64 = vmm
            .cpu_state(cpu_id)
            .unwrap()
            .read_reg(Register::RBX)
            .unwrap();
        assert_eq!(rbx, target_rax);

        // Check that rax is still at 0x100
        let new_rax: u64 = vmm
            .cpu_state(cpu_id)
            .unwrap()
            .read_reg(Register::RAX)
            .unwrap();
        assert_eq!(rax, new_rax);

        Ok(())
    }
}
