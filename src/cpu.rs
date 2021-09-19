use std::sync::{Arc, Mutex, Condvar};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::{thread, time};
use crate::bus::{Bus, Channel, Memory32, BusError};

pub const PC: usize = 15;
pub const LR: usize = 14;

pub const PS: usize = 7;
pub const LS: usize = 6;

pub const SUPERVISOR_ACCESS: i32 = -1;

// functions for instruction decode
fn rr_reg_d(iword: u16) -> usize {
	((iword & 0xF0) >> 4) as usize
}

fn rr_reg_r(iword: u16) -> usize {
	(iword & 0x0F) as usize
}

fn rm_seg_s(iword: u16) -> usize {
	((iword & 0xF000) >> 12) as usize
}

fn rmx_reg_x(iword: u16) -> usize {
	((iword & 0xF00) >> 8) as usize
}

fn rmx_idx_i(iword: u16) -> u8 {
	(iword & 0xFF) as u8
}

#[allow(dead_code)]
#[allow(non_snake_case)]
pub struct SeriesQ {
	pub R: [u32; 16],
	
	pub S_selector: [u8; 16],
	pub S_base: [u32; 16],
	pub S_limit: [u32; 16],
	pub S_key: [u8; 16],
	pub S_flags: [u8; 16], // .......U (..., Unsigned RM Offsets)
	
	pub MPK: [u8; 16],
	
	pub F: [u8; 16], // F0: PLGEVCSB; F8: .......A (..., Application State)
	
	pub SDTR_base: u32,
	pub SDTR_len: u8,
	
	pub running: Arc<AtomicBool>,
	pub cycles: u64,
	
	pub bus: Arc<Mutex<Bus>>,
	pub channels: Vec<Channel<Bus>>
}

fn sign_u32(x: u32) -> bool {
	if x & 0x80000000 != 0 {
		true
	} else {
		false
	}
}

fn alu_shl(dest: u32, src: u32, flags: u8) -> (u32, u8) {
	let x = (dest as u64) << (src & 31);
	let carry = (x >> 32) & 1;
	let y = (x & 0xFFFFFFFF) as u32;
	
	let mut new_flags = flags;
	// PLGEVCSB
	if y & 1 == 1 {
		// odd
		new_flags |= 0b10000000;
	} else {
		// even
		new_flags &= 0b01111111;
	}
	
	if carry == 1 {
		new_flags |= 0b00000100;
	} else {
		new_flags &= 0b11111011;
	}
	
	(y, new_flags)
}

fn alu_shr(dest: u32, src: u32, flags: u8) -> (u32, u8) {
	let x = ((dest as u64) << 32) >> (src & 31);
	let carry = x & 0x80000000;
	let y = ((x >> 32) & 0xFFFFFFFF) as u32;
	
	let mut new_flags = flags;
	// PLGEVCSB
	if y & 1 == 1 {
		// odd
		new_flags |= 0b10000000;
	} else {
		// even
		new_flags &= 0b01111111;
	}
	
	if carry != 0 {
		new_flags |= 0b00000100;
	} else {
		new_flags &= 0b11111011;
	}
	
	(y, new_flags)
}

fn alu_sal(dest: u32, src: u32, flags: u8) -> (u32, u8) {
	let x = (dest as i64) << (src & 31);
	let carry = (x >> 32) & 1;
	let y = (x & 0xFFFFFFFF) as u32;
	
	let mut new_flags = flags;
	// PLGEVCSB
	if y & 1 == 1 {
		// odd
		new_flags |= 0b10000000;
	} else {
		// even
		new_flags &= 0b01111111;
	}
	
	if carry == 1 {
		new_flags |= 0b00000100;
	} else {
		new_flags &= 0b11111011;
	}
	
	(y, new_flags)
}

fn alu_sar(dest: u32, src: u32, flags: u8) -> (u32, u8) {
	let x = ((dest as i64) << 32) >> (src & 31);
	let carry = x & 0x80000000;
	let y = ((x >> 32) & 0xFFFFFFFF) as u32;
	
	let mut new_flags = flags;
	// PLGEVCSB
	if y & 1 == 1 {
		// odd
		new_flags |= 0b10000000;
	} else {
		// even
		new_flags &= 0b01111111;
	}
	
	if carry != 0 {
		new_flags |= 0b00000100;
	} else {
		new_flags &= 0b11111011;
	}
	
	(y, new_flags)
}

fn alu_add(dest: u32, src: u32, flags: u8, use_carry: bool) -> (u32, u8) {
	let (mut y, mut carry) = dest.overflowing_add(src);
	if flags & 0b00000100 != 0 && use_carry {
		let (z, carry_2) = y.overflowing_add(1);
		y = z;
		carry = carry && carry_2;
	}
	
	let mut new_flags = flags;
	// PLGEVCSB
	if y & 1 == 1 {
		// odd
		new_flags |= 0b10000000;
	} else {
		// even
		new_flags &= 0b01111111;
	}
	
	if src < dest {
		// less
		new_flags |= 0b01000000;
		new_flags &= 0b11001111;
	} else if src > dest {
		// greater
		new_flags |= 0b00100000;
		new_flags &= 0b10101111;
	} else {
		// equal
		new_flags |= 0b00010000;
		new_flags &= 0b10011111;
	}
	
	if (sign_u32(src) && sign_u32(dest) && !(sign_u32(y)))
		|| (!(sign_u32(src)) && !(sign_u32(dest)) && sign_u32(y)) {
		// overflow
		new_flags |= 0b00001000;
	} else {
		// no overflow
		new_flags &= 0b11110111;
	}
	
	if carry {
		new_flags |= 0b00000100;
	} else {
		new_flags &= 0b11111011;
	}
	
	if (src as i32) < (dest as i32) {
		// less
		new_flags |= 0b00000010;
		new_flags &= 0b11111110;
	} else if (src as i32) > (dest as i32) {
		// greater
		new_flags |= 0b00000001;
		new_flags &= 0b11111101;
	} else {
		new_flags &= 0b11111100;
	}
	
	(y, new_flags)
}

fn alu_sub(dest: u32, src: u32, flags: u8, use_carry: bool) -> (u32, u8) {
	let (mut y, mut carry) = dest.overflowing_sub(src);
	if flags & 0b00000100 != 0 && use_carry {
		let (z, carry_2) = y.overflowing_sub(1);
		y = z;
		carry = carry && carry_2;
	}
	
	let mut new_flags = flags;
	// PLGEVCSB
	if y & 1 == 1 {
		// odd
		new_flags |= 0b10000000;
	} else {
		// even
		new_flags &= 0b01111111;
	}
	
	if src < dest {
		// less
		new_flags |= 0b01000000;
		new_flags &= 0b11001111;
	} else if src > dest {
		// greater
		new_flags |= 0b00100000;
		new_flags &= 0b10101111;
	} else {
		// equal
		new_flags |= 0b00010000;
		new_flags &= 0b10011111;
	}
	
	if (sign_u32(src) && !(sign_u32(dest)) && sign_u32(y))
		|| (!(sign_u32(src)) && sign_u32(dest) && !(sign_u32(y))) {
		// overflow
		new_flags |= 0b00001000;
	} else {
		// no overflow
		new_flags &= 0b11110111;
	}
	
	if carry {
		new_flags |= 0b00000100;
	} else {
		new_flags &= 0b11111011;
	}
	
	if (src as i32) < (dest as i32) {
		// less
		new_flags |= 0b00000010;
		new_flags &= 0b11111110;
	} else if (src as i32) > (dest as i32) {
		// greater
		new_flags |= 0b00000001;
		new_flags &= 0b11111101;
	} else {
		new_flags &= 0b11111100;
	}
	
	(y, new_flags)
}

pub trait SQAddr {
	fn gen_offset_rm(&self, reg_segment: usize, reg_base: usize, index: u16) -> u32;
	fn gen_offset_rmx(&self, reg_segment: usize, reg_base: usize, reg_offset: usize, index: u8) -> u32;
	fn gen_addr_rm(&self, reg_segment: usize, reg_base: usize, index: u16) -> u32;
	fn gen_addr_rmx(&self, reg_segment: usize, reg_base: usize,
		reg_offset: usize, index: u8) -> u32;
	fn access_check(&self, segment: usize, addr: u32, write: bool, exec: bool) -> bool;
}

impl SQAddr for SeriesQ	{
	fn gen_offset_rm(&self, reg_segment: usize, reg_base: usize, index: u16) -> u32 {
		let index: u16 = index & 0xFFF;
		let base: u32 = self.R[reg_base];
		let offset: u32 = if index & 0xFFF > 2047 && self.S_flags[reg_segment] & 1 == 0 {
			(index as u32) | 0xFFFFF000
		} else {
			index as u32
		};
		
		return base.wrapping_add(offset); // no bounds checking - 
										  // this should be done separately
	}
	
	fn gen_offset_rmx(&self, reg_segment: usize, reg_base: usize,
		reg_offset: usize, index: u8) -> u32 {
		let base: u32 = self.R[reg_base];
		let offset: u32 = self.R[reg_offset].wrapping_add(index as u32);
		return base.wrapping_add(offset);
	}

	fn gen_addr_rm(&self, reg_segment: usize, reg_base: usize, index: u16) -> u32 {
		let base: u32 = self.S_base[reg_segment];
		let offset = self.gen_offset_rm(reg_segment, reg_base, index & 0xFFF);
		
		return base.wrapping_add(offset); // no bounds checking - 
										  // this should be done separately
	}
	
	fn gen_addr_rmx(&self, reg_segment: usize, reg_base: usize,
		reg_offset: usize, index: u8) -> u32 {
		let base: u32 = self.S_base[reg_segment];
		let offset = self.gen_offset_rmx(reg_segment, reg_base, reg_offset, index);
		return base.wrapping_add(offset);
	}
	
	fn access_check(&self, segment: usize, addr: u32, write: bool, exec: bool) -> bool {
		(self.MPK.contains(&self.S_key[segment]) || &self.F[8] & 1 == 0)
			&& addr >= self.S_base[segment]
			&& addr < self.S_limit[segment]
	}
}

impl SeriesQ {
	fn copy_segment(&mut self, dest: usize, src: usize) {
		self.S_selector[dest] = self.S_selector[src];
		self.S_base[dest] = self.S_base[src];
		self.S_limit[dest] = self.S_limit[src];
		self.S_key[dest] = self.S_key[src];
		self.S_flags[dest] = self.S_flags[src];
	}
	
	fn increment(&self, iword: u16) -> u32 {
		if (iword >> 14) & 3 == 1 || (iword >> 14) & 3 == 3 {
			4
		} else {
			2
		}
	}
	
	fn read_fault(&mut self, iword0: u16, addr: u32) {
		println!("@{:08X}::{:08X} 0x{:04X} READ FAULT 0x{:08X}", self.S_base[PS], self.R[PC], iword0, addr);
		self.running.store(false, Ordering::Relaxed);
	}
	fn write_fault(&self, iword0: u16, addr: u32) {
		println!("@{:08X}::{:08X} 0x{:04X} WRITE FAULT 0x{:08X}", self.S_base[PS], self.R[PC], iword0, addr);
		self.running.store(false, Ordering::Relaxed);
	}
	fn seg_fault(&self, iword0: u16, addr: u32) {
		println!("@{:08X}::{:08X} 0x{:04X} SEGMENTATION FAULT 0x{:08X}", self.S_base[PS], self.R[PC], iword0, addr);
		self.running.store(false, Ordering::Relaxed);
	}
	fn app_fault(&self, iword0: u16, error_code: u32) {
		println!("@{:08X}::{:08X} 0x{:04X} APPLICATION FAULT 0x{:08X}", self.S_base[PS], self.R[PC], iword0, error_code);
		self.running.store(false, Ordering::Relaxed);
	}
	fn sys_fault(&self, iword0: u16, error_code: u32) {
		println!("@{:08X}::{:08X} 0x{:04X} SYSTEM FAULT 0x{:08X}", self.S_base[PS], self.R[PC], iword0, error_code);
		self.running.store(false, Ordering::Relaxed);
	}
	
	pub fn new(bus: Arc<Mutex<Bus>>) -> SeriesQ {
		let mut result = SeriesQ {
			R: [0; 16],
			
			S_selector: [0; 16],
			S_base: [0; 16],
			S_limit: [0xFFFFFFFF; 16],
			S_key: [0xFF; 16],
			S_flags: [0xFF,
					  0xFF,
					  0xFF,
					  0xFF,
					  0xFF,
					  0xFF,
					  0xFF,
					  0xF0,
					  0xFF,
					  0xFF,
					  0xFF,
					  0xFF,
					  0xFF,
					  0xFF,
					  0xFF,
					  0xF0],
			
			MPK: [0xFF; 16],
			
			F: [0; 16],
			
			SDTR_base: 0,
			SDTR_len: 0,
			
			running: Arc::new(AtomicBool::new(false)),
			cycles: 0,
			
			bus: bus,
			channels: Vec::new()
			
		};
		
		for _ in 0..16 {
			result.channels.push(Channel::new(&result.bus));
		}
		
		result
	}
	
	pub fn run(cpu: Arc<Mutex<SeriesQ>>) {
		thread::spawn(move || {
			let mut cpu = cpu.lock().unwrap();
			cpu.cycles = 0;
			let mut skip = false;
			
			let mut our_bus = Arc::clone(&cpu.bus);
			let mut held_bus = our_bus.lock().unwrap();
			println!("CPU START, {} devices attached to bus", held_bus.region.len());
			cpu.running.store(true, Ordering::Relaxed);
			while cpu.running.load(Ordering::Relaxed) {
				// clear zero register
				cpu.R[0] = 0;
				
				// TODO: service interrupts
				
				// instruction fetch
				let mut iword0: u16 = 0;
				let mut iword1: u16 = 0;
				let mut ifetch = true;
				
				let addr = cpu.R[PC].wrapping_add(cpu.S_base[PS]);
				if cpu.access_check(PS, addr, false, true) {
					match held_bus.read_h_big(cpu.R[PC].wrapping_add(cpu.S_base[PS])) {
						Err(_) => {
							// TODO: handle read fault
							println!("@{:08X}::{:08X} READ FAULT IFETCH", cpu.S_base[PS], cpu.R[PC]);
							ifetch = false;
							// for now
							cpu.running.store(false, Ordering::Relaxed);
						},
						// TODO: increment PC after fetch as soon as we're done profiling performance
						Ok(x) => { iword0 = x; cpu.R[PC] = cpu.R[PC].wrapping_add(2); },
					};
				} else {
					// TODO: handle segmentation fault
					println!("@{:08X}::{:08X} SEGMENTATION FAULT IFETCH", cpu.S_base[PS], cpu.R[PC]);
					ifetch = false;
					// for now
					cpu.running.store(false, Ordering::Relaxed);
				}
				
				// TODO: fetch rest of instruction
				
				if ifetch && cpu.increment(iword0) >= 4 {
					let addr = cpu.R[PC].wrapping_add(cpu.S_base[PS]);
					if cpu.access_check(PS, addr, false, true) {
						match held_bus.read_h_big(cpu.R[PC].wrapping_add(cpu.S_base[PS])) {
							Err(_) => {
								// TODO: handle read fault
								println!("@{:08X}::{:08X} READ FAULT IFETCH", cpu.S_base[PS], cpu.R[PC]);
								ifetch = false;
								// for now
								cpu.running.store(false, Ordering::Relaxed);
							},
							Ok(x) => { iword1 = x; cpu.R[PC] = cpu.R[PC].wrapping_add(2); },
						};
					} else {
						// TODO: handle segmentation fault
						println!("@{:08X}::{:08X} SEGMENTATION FAULT IFETCH", cpu.S_base[PS], cpu.R[PC]);
						ifetch = false;
						// for now
						cpu.running.store(false, Ordering::Relaxed);
					}
				}
				
				if ifetch && !skip {
					// TODO: execute instructions
					match (iword0 & 0xFF00) >> 8 {
						
						// RR
						0b00000000 => { // MV, move registers
							cpu.R[rr_reg_d(iword0)] = cpu.R[rr_reg_r(iword0)];
						},
						
						0b00000001 => { // LQ, load quick
							cpu.R[rr_reg_d(iword0)] = rr_reg_r(iword0) as u32;
						},
						
						0b00000010 => { // BTR, byte truncate
							cpu.R[rr_reg_d(iword0)] = cpu.R[rr_reg_r(iword0)] & 0xFF;
						},
						0b00000011 => { // HTR, half truncate
							cpu.R[rr_reg_d(iword0)] = cpu.R[rr_reg_r(iword0)] & 0xFFFF;
						},
						
						0b00000100 => { // BSF, byte sign extend
							cpu.R[rr_reg_d(iword0)] = cpu.R[rr_reg_r(iword0)] & 0xFF;
							if cpu.R[rr_reg_r(iword0)] & 0b10000000 != 0 { // sign bit set
								cpu.R[rr_reg_d(iword0)] |= 0xFFFFFF00;
							}
						},
						0b00000101 => { // HSF, half sign extend
							cpu.R[rr_reg_d(iword0)] = cpu.R[rr_reg_r(iword0)] & 0xFFFF;
							if cpu.R[rr_reg_r(iword0)] & 0b10000000_00000000 != 0 { // sign bit set
								cpu.R[rr_reg_d(iword0)] |= 0xFFFF0000;
							}
						},
						
						0b00000110 => { // BNS, byte insert
							cpu.R[rr_reg_d(iword0)] = (cpu.R[rr_reg_d(iword0)] & 0xFFFFFF00) | (cpu.R[rr_reg_r(iword0)] & 0xFF);
						},
						0b00000111 => { // HNS, half insert
							cpu.R[rr_reg_d(iword0)] = (cpu.R[rr_reg_d(iword0)] & 0xFFFF0000) | (cpu.R[rr_reg_r(iword0)] & 0xFFFF);
						},
						
						0b00001000 => { // A, add
							let (x, flags) = alu_add(cpu.R[rr_reg_d(iword0)], cpu.R[rr_reg_r(iword0)], cpu.F[0], false);
							cpu.R[rr_reg_d(iword0)] = x;
							cpu.F[0] = flags;
						},
						0b00001001 => { // AC, add with carry
							let (x, flags) = alu_add(cpu.R[rr_reg_d(iword0)], cpu.R[rr_reg_r(iword0)], cpu.F[0], true);
							cpu.R[rr_reg_d(iword0)] = x;
							cpu.F[0] = flags;
						},
						0b00001010 => { // S, subtract
							let (x, flags) = alu_sub(cpu.R[rr_reg_d(iword0)], cpu.R[rr_reg_r(iword0)], cpu.F[0], false);
							cpu.R[rr_reg_d(iword0)] = x;
							cpu.F[0] = flags;
						},
						0b00001011 => { // SC, subtract with carry
							let (x, flags) = alu_sub(cpu.R[rr_reg_d(iword0)], cpu.R[rr_reg_r(iword0)], cpu.F[0], true);
							cpu.R[rr_reg_d(iword0)] = x;
							cpu.F[0] = flags;
						},
						
						0b00001100 => { // AQ, add quick
							let (x, flags) = alu_add(cpu.R[rr_reg_d(iword0)], rr_reg_r(iword0) as u32, cpu.F[0], false);
							cpu.R[rr_reg_d(iword0)] = x;
							cpu.F[0] = flags;
						},
						0b00001101 => { // AQC, add quick with carry
							let (x, flags) = alu_add(cpu.R[rr_reg_d(iword0)], rr_reg_r(iword0) as u32, cpu.F[0], true);
							cpu.R[rr_reg_d(iword0)] = x;
							cpu.F[0] = flags;
						},
						0b00001110 => { // SQ, subtract quick
							let (x, flags) = alu_sub(cpu.R[rr_reg_d(iword0)], rr_reg_r(iword0) as u32, cpu.F[0], false);
							cpu.R[rr_reg_d(iword0)] = x;
							cpu.F[0] = flags;
						},
						0b00001111 => { // SQC, subtract quick with carry
							let (x, flags) = alu_sub(cpu.R[rr_reg_d(iword0)], rr_reg_r(iword0) as u32, cpu.F[0], true);
							cpu.R[rr_reg_d(iword0)] = x;
							cpu.F[0] = flags;
						},
						
						0b00010000 => { // AN, bitwise And
							cpu.R[rr_reg_d(iword0)] &= cpu.R[rr_reg_r(iword0)];
						},
						0b00010001 => { // O, bitwise Or
							cpu.R[rr_reg_d(iword0)] |= cpu.R[rr_reg_r(iword0)];
						},
						0b00010010 => { // X, bitwise Xor
							cpu.R[rr_reg_d(iword0)] ^= cpu.R[rr_reg_r(iword0)];
						},
						0b00010011 => { // XN, bitwise Xnor
							cpu.R[rr_reg_d(iword0)] = !(cpu.R[rr_reg_d(iword0)] ^ cpu.R[rr_reg_r(iword0)]);
						},
						
						0b00010100 => { // ANQ, bitwise And quick
							cpu.R[rr_reg_d(iword0)] &= rr_reg_r(iword0) as u32;
						},
						0b00010101 => { // OQ, bitwise Or quick
							cpu.R[rr_reg_d(iword0)] |= rr_reg_r(iword0) as u32;
						},
						0b00010110 => { // XQ, bitwise Xor quick
							cpu.R[rr_reg_d(iword0)] ^= rr_reg_r(iword0) as u32;
						},
						0b00010111 => { // XNQ, bitwise Xnor quick
							cpu.R[rr_reg_d(iword0)] = !(cpu.R[rr_reg_d(iword0)] ^ rr_reg_r(iword0) as u32);
						},
						
						0b00011000 => { // SL, logical shift left
							let (x, flags) = alu_shl(cpu.R[rr_reg_d(iword0)], cpu.R[rr_reg_r(iword0)], cpu.F[0]);
							cpu.R[rr_reg_d(iword0)] = x;
							cpu.F[0] = flags;
						},
						0b00011001 => { // SR, logical shift right
							let (x, flags) = alu_shr(cpu.R[rr_reg_d(iword0)], cpu.R[rr_reg_r(iword0)], cpu.F[0]);
							cpu.R[rr_reg_d(iword0)] = x;
							cpu.F[0] = flags;
						},
						0b00011010 => { // ASL, arithmetic shift left
							let (x, flags) = alu_sal(cpu.R[rr_reg_d(iword0)], cpu.R[rr_reg_r(iword0)], cpu.F[0]);
							cpu.R[rr_reg_d(iword0)] = x;
							cpu.F[0] = flags;
						},
						0b00011011 => { // ASR, arithmetic shift right
							let (x, flags) = alu_sar(cpu.R[rr_reg_d(iword0)], cpu.R[rr_reg_r(iword0)], cpu.F[0]);
							cpu.R[rr_reg_d(iword0)] = x;
							cpu.F[0] = flags;
						},
						
						0b00011100 => { // SLQ, logical quick shift left
							let (x, flags) = alu_shl(cpu.R[rr_reg_d(iword0)], rr_reg_r(iword0) as u32 + 1, cpu.F[0]);
							cpu.R[rr_reg_d(iword0)] = x;
							cpu.F[0] = flags;
						},
						0b00011101 => { // SRQ, logical quick shift right
							let (x, flags) = alu_shr(cpu.R[rr_reg_d(iword0)], rr_reg_r(iword0) as u32 + 1, cpu.F[0]);
							cpu.R[rr_reg_d(iword0)] = x;
							cpu.F[0] = flags;
						},
						0b00011110 => { // ASLQ, arithmetic quick shift left
							let (x, flags) = alu_sal(cpu.R[rr_reg_d(iword0)], rr_reg_r(iword0) as u32 + 1, cpu.F[0]);
							cpu.R[rr_reg_d(iword0)] = x;
							cpu.F[0] = flags;
						},
						0b00011111 => { // ASRQ, arithmetic quick shift right
							let (x, flags) = alu_sar(cpu.R[rr_reg_d(iword0)], rr_reg_r(iword0) as u32 + 1, cpu.F[0]);
							cpu.R[rr_reg_d(iword0)] = x;
							cpu.F[0] = flags;
						},
						
						0b00100000 => { // C, compare
							let (x, flags) = alu_sub(cpu.R[rr_reg_d(iword0)], cpu.R[rr_reg_r(iword0)], cpu.F[0], false);
							// cpu.R[rr_reg_d(iword0)] = x;
							cpu.F[0] = flags;
						},
						
						0b00100010 => { // LF, load flag registers
							cpu.R[rr_reg_d(iword0)] = cpu.F[rr_reg_r(iword0)] as u32;
						},
						0b00100011 => { // SF, save flag registers
							if cpu.F[8] & 0b00000001 != 0 && rr_reg_r(iword0) >= 8 {
								// TODONE: handle application fault
								// println!("@{:08X}::{:08X} APPLICATION FAULT SF", cpu.S_base[PS], cpu.R[PC]);
								// for now
								// cpu.running.store(false, Ordering::Relaxed);
								
								cpu.app_fault(iword0, SUPERVISOR_ACCESS as u32);
							} else {
								cpu.F[rr_reg_d(iword0)] = (cpu.R[rr_reg_r(iword0)] & 0xFF) as u8;
							}
						},
						
						0b00100100 => { // LSDTR, load Segment Descriptor Table registers 
							if cpu.F[8] & 0b00000001 != 0 {
								cpu.app_fault(iword0, SUPERVISOR_ACCESS as u32);
							} else {
								cpu.R[rr_reg_r(iword0)] = cpu.SDTR_len as u32;
								cpu.R[rr_reg_d(iword0)] = cpu.SDTR_base;
							}
						},
						0b00100101 => { // SSDTR, set Segment Descriptor Table registers 
							if cpu.F[8] & 0b00000001 != 0 {
								cpu.app_fault(iword0, SUPERVISOR_ACCESS as u32);
							} else {
								cpu.SDTR_len = (cpu.R[rr_reg_r(iword0)] & 0xFF) as u8;
								cpu.SDTR_base = cpu.R[rr_reg_d(iword0)];
							}
						},
						
						0b00100110 => { // LSEL, load segment selector
							cpu.R[rr_reg_d(iword0)] = cpu.S_selector[rr_reg_r(iword0)] as u32;
						}
						0b00100111 => { // SSEL, set segment selector
							if (cpu.F[8] & 0b00000001 != 0 && rr_reg_d(iword0) >= 8) || ((cpu.R[rr_reg_r(iword0)] & 0xFF) as u8) > cpu.SDTR_len {
								cpu.app_fault(iword0, SUPERVISOR_ACCESS as u32);
							} else {
								cpu.S_selector[rr_reg_d(iword0)] = (cpu.R[rr_reg_r(iword0)] & 0xFF) as u8;
								
								// ugh
								let mut ok = true;
								
								// read S_base
								let addr = cpu.SDTR_base + 12 * (cpu.R[rr_reg_r(iword0)] & 0xFF);
								match held_bus.read_w(addr) {
									Err(_) => {
										cpu.read_fault(iword0, addr);
										ok = false;
									},
									Ok(x) => { cpu.S_base[rr_reg_d(iword0)] = x; },
								};
								
								if ok {
									// read S_limit
									let addr = cpu.SDTR_base + 12 * (cpu.R[rr_reg_r(iword0)] & 0xFF) + 4;
									match held_bus.read_w(addr) {
										Err(_) => {
											cpu.read_fault(iword0, addr);
											ok = false;
										},
										Ok(x) => { cpu.S_limit[rr_reg_d(iword0)] = x; },
									};
								}
								
								if ok {
									// read S_key
									let addr = cpu.SDTR_base + 12 * (cpu.R[rr_reg_r(iword0)] & 0xFF) + 8;
									match held_bus.read_b(addr) {
										Err(_) => {
											cpu.read_fault(iword0, addr);
											ok = false;
										},
										Ok(x) => { cpu.S_key[rr_reg_d(iword0)] = x; },
									};
								}
								
								if ok {
									// read S_flags
									let addr = cpu.SDTR_base + 12 * (cpu.R[rr_reg_r(iword0)] & 0xFF) + 9;
									match held_bus.read_b(addr) {
										Err(_) => {
											cpu.read_fault(iword0, addr);
											ok = false;
										},
										Ok(x) => { cpu.S_flags[rr_reg_d(iword0)] = x; },
									};
								}
								
							}
						}
						
						0b00101000 => { // LMPK, get memory protection key
							if (cpu.F[8] & 0b00000001 != 0) {
								cpu.app_fault(iword0, SUPERVISOR_ACCESS as u32);
							} else {
								cpu.R[rr_reg_d(iword0)] = cpu.MPK[rr_reg_r(iword0)] as u32;
							}
						}
						0b00101001 => { // SMPK, get memory protection key
							if (cpu.F[8] & 0b00000001 != 0) {
								cpu.app_fault(iword0, SUPERVISOR_ACCESS as u32);
							} else {
								cpu.MPK[rr_reg_d(iword0)] = cpu.R[rr_reg_r(iword0)] as u8;
							}
						}
						
						0b00101010 => { // CSEL, copy segment selector
							if (cpu.F[8] & 0b00000001 != 0 && rr_reg_d(iword0) >= 8) {
								cpu.app_fault(iword0, SUPERVISOR_ACCESS as u32);
							} else {
								cpu.copy_segment(rr_reg_d(iword0), rr_reg_r(iword0));
							}
						}
						0b00101011 => { // SSELHC, set segment selector
							if (cpu.F[8] & 0b00000001 != 0 && rr_reg_d(iword0) >= 8) || ((rr_reg_r(iword0) & 0xFF) as u8) > cpu.SDTR_len {
								cpu.app_fault(iword0, SUPERVISOR_ACCESS as u32);
							} else {
								cpu.S_selector[rr_reg_d(iword0)] = ((rr_reg_r(iword0) as u32) & 0xFF) as u8;
								
								// ugh
								let mut ok = true;
								
								// read S_base
								let addr = cpu.SDTR_base + 12 * ((rr_reg_r(iword0) as u32) & 0xFF);
								match held_bus.read_w(addr) {
									Err(_) => {
										cpu.read_fault(iword0, addr);
										ok = false;
									},
									Ok(x) => { cpu.S_base[rr_reg_d(iword0)] = x; },
								};
								
								if ok {
									// read S_limit
									let addr = cpu.SDTR_base + 12 * ((rr_reg_r(iword0) as u32) & 0xFF) + 4;
									match held_bus.read_w(addr) {
										Err(_) => {
											cpu.read_fault(iword0, addr);
											ok = false;
										},
										Ok(x) => { cpu.S_limit[rr_reg_d(iword0)] = x; },
									};
								}
								
								if ok {
									// read S_key
									let addr = cpu.SDTR_base + 12 * ((rr_reg_r(iword0) as u32) & 0xFF) + 8;
									match held_bus.read_b(addr) {
										Err(_) => {
											cpu.read_fault(iword0, addr);
											ok = false;
										},
										Ok(x) => { cpu.S_key[rr_reg_d(iword0)] = x; },
									};
								}
								
								if ok {
									// read S_flags
									let addr = cpu.SDTR_base + 12 * ((rr_reg_r(iword0) as u32) & 0xFF) + 9;
									match held_bus.read_b(addr) {
										Err(_) => {
											cpu.read_fault(iword0, addr);
											ok = false;
										},
										Ok(x) => { cpu.S_flags[rr_reg_d(iword0)] = x; },
									};
								}
								
							}
						}
						
						0b00111110 => { // IF, conditionally execute next instruction
							let mask = (iword0 & 0xFF) as u8;
							if mask & cpu.F[0] == 0 {
								skip = true;
							}
						},
						0b00111111 => { // IFN, conditionally skip next instruction
							let mask = (iword0 & 0xFF) as u8;
							if mask & cpu.F[0] != 0 {
								skip = true;
							}
						},
						
						// RMX
						0b01000000 => { // RMX L, load word
							let addr = cpu.gen_addr_rmx(rm_seg_s(iword1), rr_reg_r(iword0), rmx_reg_x(iword1), rmx_idx_i(iword1));
							if cpu.access_check(rm_seg_s(iword1), addr, false, false) {
								match held_bus.read_w(addr) {
									Err(_) => {
										cpu.read_fault(iword0, addr);
									},
									Ok(x) => { cpu.R[rr_reg_d(iword0)] = x; },
								};
							} else {
								cpu.seg_fault(iword0, addr);
							}
						},
						0b01000001 => { // RMX LA, load address
							cpu.R[rr_reg_d(iword0)] = cpu.gen_offset_rmx(rm_seg_s(iword1), rr_reg_r(iword0), rmx_reg_x(iword1), rmx_idx_i(iword1));
						},
						
						0b01000010 => { // RMX BTR, byte truncate
							let addr = cpu.gen_addr_rmx(rm_seg_s(iword1), rr_reg_r(iword0), rmx_reg_x(iword1), rmx_idx_i(iword1));
							if cpu.access_check(rm_seg_s(iword1), addr, false, false) {
								match held_bus.read_b(addr) {
									Err(_) => {
										cpu.read_fault(iword0, addr);
									},
									Ok(x) => { cpu.R[rr_reg_d(iword0)] = x as u32; },
								};
							} else {
								cpu.seg_fault(iword0, addr);
							}
						},
						0b01000011 => { // RMX HTR, half truncate
							let addr = cpu.gen_addr_rmx(rm_seg_s(iword1), rr_reg_r(iword0), rmx_reg_x(iword1), rmx_idx_i(iword1));
							if cpu.access_check(rm_seg_s(iword1), addr, false, false) {
								match held_bus.read_h(addr) {
									Err(_) => {
										cpu.read_fault(iword0, addr);
									},
									Ok(x) => { cpu.R[rr_reg_d(iword0)] = x as u32; },
								};
							} else {
								cpu.seg_fault(iword0, addr);
							}
						},
						
						0b01000100 => { // RMX BSF, byte sign extend
							let addr = cpu.gen_addr_rmx(rm_seg_s(iword1), rr_reg_r(iword0), rmx_reg_x(iword1), rmx_idx_i(iword1));
							if cpu.access_check(rm_seg_s(iword1), addr, false, false) {
								match held_bus.read_b(addr) {
									Err(_) => {
										cpu.read_fault(iword0, addr);
									},
									Ok(x) => {
										cpu.R[rr_reg_d(iword0)] = x as u32;
										if x & 0b10000000 != 0 { // sign bit set
											cpu.R[rr_reg_d(iword0)] |= 0xFFFFFF00;
										}
									},
								};
							} else {
								cpu.seg_fault(iword0, addr);
							}
						},
						0b01000101 => { // RMX HSF, half sign extend
							let addr = cpu.gen_addr_rmx(rm_seg_s(iword1), rr_reg_r(iword0), rmx_reg_x(iword1), rmx_idx_i(iword1));
							if cpu.access_check(rm_seg_s(iword1), addr, false, false) {
								match held_bus.read_h(addr) {
									Err(_) => {
										cpu.read_fault(iword0, addr);
									},
									Ok(x) => {
										cpu.R[rr_reg_d(iword0)] = x as u32;
										if x & 0b10000000_00000000 != 0 { // sign bit set
											cpu.R[rr_reg_d(iword0)] |= 0xFFFF0000;
										}
									},
								};
							} else {
								cpu.seg_fault(iword0, addr);
							}
						},
						
						0b01000110 => { // RMX BNS, byte insert
							let addr = cpu.gen_addr_rmx(rm_seg_s(iword1), rr_reg_r(iword0), rmx_reg_x(iword1), rmx_idx_i(iword1));
							if cpu.access_check(rm_seg_s(iword1), addr, false, false) {
								match held_bus.read_b(addr) {
									Err(_) => {
										cpu.read_fault(iword0, addr);
									},
									Ok(x) => {
										cpu.R[rr_reg_d(iword0)] = (cpu.R[rr_reg_d(iword0)] & 0xFFFFFF00) | (x as u32);
									},
								};
							} else {
								cpu.seg_fault(iword0, addr);
							}
						},
						0b01000111 => { // RMX HNS, half insert
							let addr = cpu.gen_addr_rmx(rm_seg_s(iword1), rr_reg_r(iword0), rmx_reg_x(iword1), rmx_idx_i(iword1));
							if cpu.access_check(rm_seg_s(iword1), addr, false, false) {
								match held_bus.read_h(addr) {
									Err(_) => {
										cpu.read_fault(iword0, addr);
									},
									Ok(x) => {
										cpu.R[rr_reg_d(iword0)] = (cpu.R[rr_reg_d(iword0)] & 0xFFFF0000) | (x as u32);
									},
								};
							} else {
								cpu.seg_fault(iword0, addr);
							}
						},
						
						0b01001000 => { // RMX ST, store word
							let addr = cpu.gen_addr_rmx(rm_seg_s(iword1), rr_reg_r(iword0), rmx_reg_x(iword1), rmx_idx_i(iword1));
							if cpu.access_check(rm_seg_s(iword1), addr, true, false) {
								match held_bus.write_w(addr, cpu.R[rr_reg_d(iword0)]) {
									Err(_) => {
										cpu.write_fault(iword0, addr);
									},
									Ok(_) => { /* do nothing */ },
								};
							} else {
								cpu.seg_fault(iword0, addr);
							}
						},
						0b01001001 => { // RMX BST, store byte
							let addr = cpu.gen_addr_rmx(rm_seg_s(iword1), rr_reg_r(iword0), rmx_reg_x(iword1), rmx_idx_i(iword1));
							if cpu.access_check(rm_seg_s(iword1), addr, true, false) {
								match held_bus.write_b(addr, (cpu.R[rr_reg_d(iword0)] & 0xFF) as u8) {
									Err(_) => {
										cpu.write_fault(iword0, addr);
									},
									Ok(_) => { /* do nothing */ },
								};
							} else {
								cpu.seg_fault(iword0, addr);
							}
						},
						0b01001010 => { // RMX HST, store half
							let addr = cpu.gen_addr_rmx(rm_seg_s(iword1), rr_reg_r(iword0), rmx_reg_x(iword1), rmx_idx_i(iword1));
							if cpu.access_check(rm_seg_s(iword1), addr, true, false) {
								match held_bus.write_h(addr, (cpu.R[rr_reg_d(iword0)] & 0xFFFF) as u16) {
									Err(_) => {
										cpu.write_fault(iword0, addr);
									},
									Ok(_) => { /* do nothing */ },
								};
							} else {
								cpu.seg_fault(iword0, addr);
							}
						},
						
						0b01011111 => { // RMX BAL, branch and optionally link
							if rr_reg_d(iword0) != 0 {
								cpu.copy_segment(LS, PS);
								cpu.R[rr_reg_d(iword0)] = cpu.R[PC];
							}
							
							cpu.copy_segment(PS, rm_seg_s(iword1));
							cpu.R[PC] = cpu.gen_offset_rmx(rm_seg_s(iword1), rr_reg_r(iword0), rmx_reg_x(iword1), rmx_idx_i(iword1));
						},
						
						// RM
						0b01100000 => { // RM L, load word
							let addr = cpu.gen_addr_rm(rm_seg_s(iword1), rr_reg_r(iword0), iword1);
							println!("{:08X}", addr);
							if cpu.access_check(rm_seg_s(iword1), addr, false, false) {
								match held_bus.read_w(addr) {
									Err(_) => {
										cpu.read_fault(iword0, addr);
									},
									Ok(x) => { cpu.R[rr_reg_d(iword0)] = x;},
								};
							} else {
								cpu.seg_fault(iword0, addr);
							}
						},
						0b01100001 => { // RM LA, load address
							cpu.R[rr_reg_d(iword0)] = cpu.gen_offset_rm(rm_seg_s(iword1), rr_reg_r(iword0), iword1);
						},
						
						0b01100010 => { // RM BTR, byte truncate
							let addr = cpu.gen_addr_rm(rm_seg_s(iword1), rr_reg_r(iword0), iword1);
							if cpu.access_check(rm_seg_s(iword1), addr, false, false) {
								match held_bus.read_b(addr) {
									Err(_) => {
										cpu.read_fault(iword0, addr);
									},
									Ok(x) => { cpu.R[rr_reg_d(iword0)] = x as u32; },
								};
							} else {
								cpu.seg_fault(iword0, addr);
							}
						},
						0b01100011 => { // RM HTR, half truncate
							let addr = cpu.gen_addr_rm(rm_seg_s(iword1), rr_reg_r(iword0), iword1);
							if cpu.access_check(rm_seg_s(iword1), addr, false, false) {
								match held_bus.read_h(addr) {
									Err(_) => {
										cpu.read_fault(iword0, addr);
									},
									Ok(x) => { cpu.R[rr_reg_d(iword0)] = x as u32; },
								};
							} else {
								cpu.seg_fault(iword0, addr);
							}
						},
						
						0b01100100 => { // RM BSF, byte sign extend
							let addr = cpu.gen_addr_rm(rm_seg_s(iword1), rr_reg_r(iword0), iword1);
							if cpu.access_check(rm_seg_s(iword1), addr, false, false) {
								match held_bus.read_b(addr) {
									Err(_) => {
										cpu.read_fault(iword0, addr);
									},
									Ok(x) => {
										cpu.R[rr_reg_d(iword0)] = x as u32;
										if x & 0b10000000 != 0 { // sign bit set
											cpu.R[rr_reg_d(iword0)] |= 0xFFFFFF00;
										}
									},
								};
							} else {
								cpu.seg_fault(iword0, addr);
							}
						},
						0b01100101 => { // RM HSF, half sign extend
							let addr = cpu.gen_addr_rm(rm_seg_s(iword1), rr_reg_r(iword0), iword1);
							if cpu.access_check(rm_seg_s(iword1), addr, false, false) {
								match held_bus.read_h(addr) {
									Err(_) => {
										cpu.read_fault(iword0, addr);
									},
									Ok(x) => {
										cpu.R[rr_reg_d(iword0)] = x as u32;
										if x & 0b10000000_00000000 != 0 { // sign bit set
											cpu.R[rr_reg_d(iword0)] |= 0xFFFF0000;
										}
									},
								};
							} else {
								cpu.seg_fault(iword0, addr);
							}
						},
						
						0b01100110 => { // RM BNS, byte insert
							let addr = cpu.gen_addr_rm(rm_seg_s(iword1), rr_reg_r(iword0), iword1);
							if cpu.access_check(rm_seg_s(iword1), addr, false, false) {
								match held_bus.read_b(addr) {
									Err(_) => {
										cpu.read_fault(iword0, addr);
									},
									Ok(x) => {
										cpu.R[rr_reg_d(iword0)] = (cpu.R[rr_reg_d(iword0)] & 0xFFFFFF00) | (x as u32);
									},
								};
							} else {
								cpu.seg_fault(iword0, addr);
							}
						},
						0b01100111 => { // RM HNS, half insert
							let addr = cpu.gen_addr_rm(rm_seg_s(iword1), rr_reg_r(iword0), iword1);
							if cpu.access_check(rm_seg_s(iword1), addr, false, false) {
								match held_bus.read_h(addr) {
									Err(_) => {
										cpu.read_fault(iword0, addr);
									},
									Ok(x) => {
										cpu.R[rr_reg_d(iword0)] = (cpu.R[rr_reg_d(iword0)] & 0xFFFF0000) | (x as u32);
									},
								};
							} else {
								cpu.seg_fault(iword0, addr);
							}
						},
						
						0b01101000 => { // RM ST, store word
							let addr = cpu.gen_addr_rm(rm_seg_s(iword1), rr_reg_r(iword0), iword1);
							if cpu.access_check(rm_seg_s(iword1), addr, true, false) {
								match held_bus.write_w(addr, cpu.R[rr_reg_d(iword0)]) {
									Err(_) => {
										cpu.write_fault(iword0, addr);
									},
									Ok(_) => { /* do nothing */ },
								};
							} else {
								cpu.seg_fault(iword0, addr);
							}
						},
						0b01101001 => { // RM BST, store byte
							let addr = cpu.gen_addr_rm(rm_seg_s(iword1), rr_reg_r(iword0), iword1);
							if cpu.access_check(rm_seg_s(iword1), addr, true, false) {
								match held_bus.write_b(addr, (cpu.R[rr_reg_d(iword0)] & 0xFF) as u8) {
									Err(_) => {
										cpu.write_fault(iword0, addr);
									},
									Ok(_) => { /* do nothing */ },
								};
							} else {
								cpu.seg_fault(iword0, addr);
							}
						},
						0b01101010 => { // RM HST, store half
							let addr = cpu.gen_addr_rm(rm_seg_s(iword1), rr_reg_r(iword0), iword1);
							if cpu.access_check(rm_seg_s(iword1), addr, true, false) {
								match held_bus.write_h(addr, (cpu.R[rr_reg_d(iword0)] & 0xFFFF) as u16) {
									Err(_) => {
										cpu.write_fault(iword0, addr);
									},
									Ok(_) => { /* do nothing */ },
								};
							} else {
								cpu.seg_fault(iword0, addr);
							}
						},
						
						0b01111111 => { // RM BAL, branch and optionally link
							if rr_reg_d(iword0) != 0 {
								cpu.copy_segment(LS, PS);
								cpu.R[rr_reg_d(iword0)] = cpu.R[PC];
							}
							
							cpu.copy_segment(PS, rm_seg_s(iword1));
							cpu.R[PC] = cpu.gen_offset_rm(rm_seg_s(iword1), rr_reg_r(iword0), iword1);
						},
						
						_ => {
							// TODO: handle illegal instruction
							println!("@{:08X}::{:08X} ILLEGAL INSTRUCTION", cpu.S_base[PS], cpu.R[PC]);
							cpu.running.store(false, Ordering::Relaxed);
						},
					};
				} else if skip {
					skip = false;
				}
				
				// TODO: service DMA
				
				for c in &cpu.channels {
					if c.check_pending() {
						drop(held_bus);
						c.open();
						held_bus = our_bus.lock().unwrap();
					}
				}
				
				cpu.cycles = cpu.cycles.wrapping_add(1);
			}
			println!("@{:08X}::{:08X} CPU STOP - {} cycles", cpu.S_base[PS], cpu.R[PC], cpu.cycles);
		});
	}
}
