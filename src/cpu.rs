use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::{thread, time};
use crate::bus::{Bus, Memory32, BusError};

pub const PC: usize = 15;
pub const LR: usize = 14;

pub const PS: usize = 15;
pub const LS: usize = 14;

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
	
	pub F: [u8; 16],
	
	pub SDTR_base: u32,
	pub SDTR_len: u8,
	
	pub running: Arc<AtomicBool>,
	pub cycles: u64
}

fn sign_u32(x: u32) -> bool {
	if x & 0x80000000 != 0 {
		true
	} else {
		false
	}
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
		self.MPK.contains(&self.S_key[segment])
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
	
	pub fn new() -> SeriesQ {
		SeriesQ {
			R: [0; 16],
			
			S_selector: [0; 16],
			S_base: [0; 16],
			S_limit: [0xFFFFFFFF; 16],
			S_key: [0xFF; 16],
			S_flags: [0xF0; 16],
			
			MPK: [0xFF; 16],
			
			F: [0; 16],
			
			SDTR_base: 0,
			SDTR_len: 0,
			
			running: Arc::new(AtomicBool::new(false)),
			cycles: 0
		}
	}
	
	pub fn run(cpu: Arc<Mutex<SeriesQ>>, bus: Arc<Mutex<Bus>>) {
		thread::spawn(move || {
			let mut cpu = cpu.lock().unwrap();
			cpu.cycles = 0;
			let mut skip = false;
			
			let mut held_bus = bus.lock().unwrap();
			println!("CPU START, {} devices attached to bus", held_bus.region.len());
			cpu.running.store(true, Ordering::Relaxed);
			while cpu.running.load(Ordering::Relaxed) {
				// clear zero register
				cpu.R[0] = 0;
				// println!("@{:08X}::{:08X}", cpu.S_base[PS], cpu.R[PC]);
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
						0b00001100 => { // C, compare
							let (x, flags) = alu_sub(cpu.R[rr_reg_d(iword0)], cpu.R[rr_reg_r(iword0)], cpu.F[0], false);
							// cpu.R[rr_reg_d(iword0)] = x;
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
										// TODO: handle read fault
										println!("@{:08X}::{:08X} READ FAULT RMX L 0x{:08X}", cpu.S_base[PS], cpu.R[PC], addr);
										// for now
										cpu.running.store(false, Ordering::Relaxed);
									},
									Ok(x) => { cpu.R[rr_reg_d(iword0)] = x; },
								};
							} else {
								// TODO: handle segmentation fault
								println!("@{:08X}::{:08X} SEGMENTATION FAULT RMX L 0x{:08X}", cpu.S_base[PS], cpu.R[PC], addr);
								// for now
								cpu.running.store(false, Ordering::Relaxed);
							}
						},
						0b01000001 => { // RMX LA, load address
							cpu.R[rr_reg_d(iword0)] = cpu.gen_offset_rmx(rm_seg_s(iword1), rr_reg_r(iword0), rmx_reg_x(iword1), rmx_idx_i(iword1));;
						},
						
						0b01000010 => { // RMX BTR, byte truncate
							let addr = cpu.gen_addr_rmx(rm_seg_s(iword1), rr_reg_r(iword0), rmx_reg_x(iword1), rmx_idx_i(iword1));
							if cpu.access_check(rm_seg_s(iword1), addr, false, false) {
								match held_bus.read_b(addr) {
									Err(_) => {
										// TODO: handle read fault
										println!("@{:08X}::{:08X} READ FAULT RMX BTR 0x{:08X}", cpu.S_base[PS], cpu.R[PC], addr);
										// for now
										cpu.running.store(false, Ordering::Relaxed);
									},
									Ok(x) => { cpu.R[rr_reg_d(iword0)] = x as u32; },
								};
							} else {
								// TODO: handle segmentation fault
								println!("@{:08X}::{:08X} SEGMENTATION FAULT RMX BTR 0x{:08X}", cpu.S_base[PS], cpu.R[PC], addr);
								// for now
								cpu.running.store(false, Ordering::Relaxed);
							}
						},
						0b01000011 => { // RMX HTR, half truncate
							let addr = cpu.gen_addr_rmx(rm_seg_s(iword1), rr_reg_r(iword0), rmx_reg_x(iword1), rmx_idx_i(iword1));
							if cpu.access_check(rm_seg_s(iword1), addr, false, false) {
								match held_bus.read_h(addr) {
									Err(_) => {
										// TODO: handle read fault
										println!("@{:08X}::{:08X} READ FAULT RMX HTR 0x{:08X}", cpu.S_base[PS], cpu.R[PC], addr);
										// for now
										cpu.running.store(false, Ordering::Relaxed);
									},
									Ok(x) => { cpu.R[rr_reg_d(iword0)] = x as u32; },
								};
							} else {
								// TODO: handle segmentation fault
								println!("@{:08X}::{:08X} SEGMENTATION FAULT RMX HTR 0x{:08X}", cpu.S_base[PS], cpu.R[PC], addr);
								// for now
								cpu.running.store(false, Ordering::Relaxed);
							}
						},
						
						0b01000100 => { // RMX BSF, byte sign extend
							let addr = cpu.gen_addr_rmx(rm_seg_s(iword1), rr_reg_r(iword0), rmx_reg_x(iword1), rmx_idx_i(iword1));
							if cpu.access_check(rm_seg_s(iword1), addr, false, false) {
								match held_bus.read_b(addr) {
									Err(_) => {
										// TODO: handle read fault
										println!("@{:08X}::{:08X} READ FAULT RMX BSF 0x{:08X}", cpu.S_base[PS], cpu.R[PC], addr);
										// for now
										cpu.running.store(false, Ordering::Relaxed);
									},
									Ok(x) => {
										cpu.R[rr_reg_d(iword0)] = x as u32;
										if x & 0b10000000 != 0 { // sign bit set
											cpu.R[rr_reg_d(iword0)] |= 0xFFFFFF00;
										}
									},
								};
							} else {
								// TODO: handle segmentation fault
								println!("@{:08X}::{:08X} SEGMENTATION FAULT RMX BSF 0x{:08X}", cpu.S_base[PS], cpu.R[PC], addr);
								// for now
								cpu.running.store(false, Ordering::Relaxed);
							}
						},
						0b01000101 => { // RMX HSF, half sign extend
							let addr = cpu.gen_addr_rmx(rm_seg_s(iword1), rr_reg_r(iword0), rmx_reg_x(iword1), rmx_idx_i(iword1));
							if cpu.access_check(rm_seg_s(iword1), addr, false, false) {
								match held_bus.read_h(addr) {
									Err(_) => {
										// TODO: handle read fault
										println!("@{:08X}::{:08X} READ FAULT RMX HSF 0x{:08X}", cpu.S_base[PS], cpu.R[PC], addr);
										// for now
										cpu.running.store(false, Ordering::Relaxed);
									},
									Ok(x) => {
										cpu.R[rr_reg_d(iword0)] = x as u32;
										if x & 0b10000000_00000000 != 0 { // sign bit set
											cpu.R[rr_reg_d(iword0)] |= 0xFFFF0000;
										}
									},
								};
							} else {
								// TODO: handle segmentation fault
								println!("@{:08X}::{:08X} SEGMENTATION FAULT RMX HSF 0x{:08X}", cpu.S_base[PS], cpu.R[PC], addr);
								// for now
								cpu.running.store(false, Ordering::Relaxed);
							}
						},
						
						0b01000110 => { // RMX BNS, byte insert
							let addr = cpu.gen_addr_rmx(rm_seg_s(iword1), rr_reg_r(iword0), rmx_reg_x(iword1), rmx_idx_i(iword1));
							if cpu.access_check(rm_seg_s(iword1), addr, false, false) {
								match held_bus.read_b(addr) {
									Err(_) => {
										// TODO: handle read fault
										println!("@{:08X}::{:08X} READ FAULT RMX BNS 0x{:08X}", cpu.S_base[PS], cpu.R[PC], addr);
										// for now
										cpu.running.store(false, Ordering::Relaxed);
									},
									Ok(x) => {
										cpu.R[rr_reg_d(iword0)] = (cpu.R[rr_reg_d(iword0)] & 0xFFFFFF00) | (x as u32);
									},
								};
							} else {
								// TODO: handle segmentation fault
								println!("@{:08X}::{:08X} SEGMENTATION FAULT RMX BNS 0x{:08X}", cpu.S_base[PS], cpu.R[PC], addr);
								// for now
								cpu.running.store(false, Ordering::Relaxed);
							}
						},
						0b01000111 => { // RMX HNS, half insert
							let addr = cpu.gen_addr_rmx(rm_seg_s(iword1), rr_reg_r(iword0), rmx_reg_x(iword1), rmx_idx_i(iword1));
							if cpu.access_check(rm_seg_s(iword1), addr, false, false) {
								match held_bus.read_h(addr) {
									Err(_) => {
										// TODO: handle read fault
										println!("@{:08X}::{:08X} READ FAULT RMX HNS 0x{:08X}", cpu.S_base[PS], cpu.R[PC], addr);
										// for now
										cpu.running.store(false, Ordering::Relaxed);
									},
									Ok(x) => {
										cpu.R[rr_reg_d(iword0)] = (cpu.R[rr_reg_d(iword0)] & 0xFFFF0000) | (x as u32);
									},
								};
							} else {
								// TODO: handle segmentation fault
								println!("@{:08X}::{:08X} SEGMENTATION FAULT RMX HNS 0x{:08X}", cpu.S_base[PS], cpu.R[PC], addr);
								// for now
								cpu.running.store(false, Ordering::Relaxed);
							}
						},
						
						0b01001000 => { // RMX ST, store word
							let addr = cpu.gen_addr_rmx(rm_seg_s(iword1), rr_reg_r(iword0), rmx_reg_x(iword1), rmx_idx_i(iword1));
							if cpu.access_check(rm_seg_s(iword1), addr, true, false) {
								match held_bus.write_w(addr, cpu.R[rr_reg_d(iword0)]) {
									Err(_) => {
										// TODO: handle write fault
										println!("@{:08X}::{:08X} WRITE FAULT RMX ST 0x{:08X}", cpu.S_base[PS], cpu.R[PC], addr);
										// for now
										cpu.running.store(false, Ordering::Relaxed);
									},
									Ok(_) => { /* do nothing */ },
								};
							} else {
								// TODO: handle segmentation fault
								println!("@{:08X}::{:08X} SEGMENTATION FAULT RMX ST 0x{:08X}", cpu.S_base[PS], cpu.R[PC], addr);
								// for now
								cpu.running.store(false, Ordering::Relaxed);
							}
						},
						0b01001001 => { // RMX BST, store byte
							let addr = cpu.gen_addr_rmx(rm_seg_s(iword1), rr_reg_r(iword0), rmx_reg_x(iword1), rmx_idx_i(iword1));
							if cpu.access_check(rm_seg_s(iword1), addr, true, false) {
								match held_bus.write_b(addr, (cpu.R[rr_reg_d(iword0)] & 0xFF) as u8) {
									Err(_) => {
										// TODO: handle write fault
										println!("@{:08X}::{:08X} WRITE FAULT RMX BST 0x{:08X}", cpu.S_base[PS], cpu.R[PC], addr);
										// for now
										cpu.running.store(false, Ordering::Relaxed);
									},
									Ok(_) => { /* do nothing */ },
								};
							} else {
								// TODO: handle segmentation fault
								println!("@{:08X}::{:08X} SEGMENTATION FAULT RMX BST 0x{:08X}", cpu.S_base[PS], cpu.R[PC], addr);
								// for now
								cpu.running.store(false, Ordering::Relaxed);
							}
						},
						0b01001010 => { // RMX HST, store half
							let addr = cpu.gen_addr_rmx(rm_seg_s(iword1), rr_reg_r(iword0), rmx_reg_x(iword1), rmx_idx_i(iword1));
							if cpu.access_check(rm_seg_s(iword1), addr, true, false) {
								match held_bus.write_h(addr, (cpu.R[rr_reg_d(iword0)] & 0xFFFF) as u16) {
									Err(_) => {
										// TODO: handle write fault
										println!("@{:08X}::{:08X} WRITE FAULT RMX HST 0x{:08X}", cpu.S_base[PS], cpu.R[PC], addr);
										// for now
										cpu.running.store(false, Ordering::Relaxed);
									},
									Ok(_) => { /* do nothing */ },
								};
							} else {
								// TODO: handle segmentation fault
								println!("@{:08X}::{:08X} SEGMENTATION FAULT RMX HST 0x{:08X}", cpu.S_base[PS], cpu.R[PC], addr);
								// for now
								cpu.running.store(false, Ordering::Relaxed);
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
							if cpu.access_check(rm_seg_s(iword1), addr, false, false) {
								match held_bus.read_w(addr) {
									Err(_) => {
										// TODO: handle read fault
										println!("@{:08X}::{:08X} READ FAULT RM L 0x{:08X}", cpu.S_base[PS], cpu.R[PC], addr);
										// for now
										cpu.running.store(false, Ordering::Relaxed);
									},
									Ok(x) => { cpu.R[rr_reg_d(iword0)] = x; },
								};
							} else {
								// TODO: handle segmentation fault
								println!("@{:08X}::{:08X} SEGMENTATION FAULT RM L 0x{:08X}", cpu.S_base[PS], cpu.R[PC], addr);
								// for now
								cpu.running.store(false, Ordering::Relaxed);
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
										// TODO: handle read fault
										println!("@{:08X}::{:08X} READ FAULT RM BTR 0x{:08X}", cpu.S_base[PS], cpu.R[PC], addr);
										// for now
										cpu.running.store(false, Ordering::Relaxed);
									},
									Ok(x) => { cpu.R[rr_reg_d(iword0)] = x as u32; },
								};
							} else {
								// TODO: handle segmentation fault
								println!("@{:08X}::{:08X} SEGMENTATION FAULT RM BTR 0x{:08X}", cpu.S_base[PS], cpu.R[PC], addr);
								// for now
								cpu.running.store(false, Ordering::Relaxed);
							}
						},
						0b01100011 => { // RM HTR, half truncate
							let addr = cpu.gen_addr_rm(rm_seg_s(iword1), rr_reg_r(iword0), iword1);
							if cpu.access_check(rm_seg_s(iword1), addr, false, false) {
								match held_bus.read_h(addr) {
									Err(_) => {
										// TODO: handle read fault
										println!("@{:08X}::{:08X} READ FAULT RM HTR 0x{:08X}", cpu.S_base[PS], cpu.R[PC], addr);
										// for now
										cpu.running.store(false, Ordering::Relaxed);
									},
									Ok(x) => { cpu.R[rr_reg_d(iword0)] = x as u32; },
								};
							} else {
								// TODO: handle segmentation fault
								println!("@{:08X}::{:08X} SEGMENTATION FAULT RM HTR 0x{:08X}", cpu.S_base[PS], cpu.R[PC], addr);
								// for now
								cpu.running.store(false, Ordering::Relaxed);
							}
						},
						
						0b01100100 => { // RM BSF, byte sign extend
							let addr = cpu.gen_addr_rm(rm_seg_s(iword1), rr_reg_r(iword0), iword1);
							if cpu.access_check(rm_seg_s(iword1), addr, false, false) {
								match held_bus.read_b(addr) {
									Err(_) => {
										// TODO: handle read fault
										println!("@{:08X}::{:08X} READ FAULT RM BSF 0x{:08X}", cpu.S_base[PS], cpu.R[PC], addr);
										// for now
										cpu.running.store(false, Ordering::Relaxed);
									},
									Ok(x) => {
										cpu.R[rr_reg_d(iword0)] = x as u32;
										if x & 0b10000000 != 0 { // sign bit set
											cpu.R[rr_reg_d(iword0)] |= 0xFFFFFF00;
										}
									},
								};
							} else {
								// TODO: handle segmentation fault
								println!("@{:08X}::{:08X} SEGMENTATION FAULT RM BSF 0x{:08X}", cpu.S_base[PS], cpu.R[PC], addr);
								// for now
								cpu.running.store(false, Ordering::Relaxed);
							}
						},
						0b01100101 => { // RM HSF, half sign extend
							let addr = cpu.gen_addr_rm(rm_seg_s(iword1), rr_reg_r(iword0), iword1);
							if cpu.access_check(rm_seg_s(iword1), addr, false, false) {
								match held_bus.read_h(addr) {
									Err(_) => {
										// TODO: handle read fault
										println!("@{:08X}::{:08X} READ FAULT RM HSF 0x{:08X}", cpu.S_base[PS], cpu.R[PC], addr);
										// for now
										cpu.running.store(false, Ordering::Relaxed);
									},
									Ok(x) => {
										cpu.R[rr_reg_d(iword0)] = x as u32;
										if x & 0b10000000_00000000 != 0 { // sign bit set
											cpu.R[rr_reg_d(iword0)] |= 0xFFFF0000;
										}
									},
								};
							} else {
								// TODO: handle segmentation fault
								println!("@{:08X}::{:08X} SEGMENTATION FAULT RM HSF 0x{:08X}", cpu.S_base[PS], cpu.R[PC], addr);
								// for now
								cpu.running.store(false, Ordering::Relaxed);
							}
						},
						
						0b01100110 => { // RM BNS, byte insert
							let addr = cpu.gen_addr_rm(rm_seg_s(iword1), rr_reg_r(iword0), iword1);
							if cpu.access_check(rm_seg_s(iword1), addr, false, false) {
								match held_bus.read_b(addr) {
									Err(_) => {
										// TODO: handle read fault
										println!("@{:08X}::{:08X} READ FAULT RM BNS 0x{:08X}", cpu.S_base[PS], cpu.R[PC], addr);
										// for now
										cpu.running.store(false, Ordering::Relaxed);
									},
									Ok(x) => {
										cpu.R[rr_reg_d(iword0)] = (cpu.R[rr_reg_d(iword0)] & 0xFFFFFF00) | (x as u32);
									},
								};
							} else {
								// TODO: handle segmentation fault
								println!("@{:08X}::{:08X} SEGMENTATION FAULT RM BNS 0x{:08X}", cpu.S_base[PS], cpu.R[PC], addr);
								// for now
								cpu.running.store(false, Ordering::Relaxed);
							}
						},
						0b01100111 => { // RM HNS, half insert
							let addr = cpu.gen_addr_rm(rm_seg_s(iword1), rr_reg_r(iword0), iword1);
							if cpu.access_check(rm_seg_s(iword1), addr, false, false) {
								match held_bus.read_h(addr) {
									Err(_) => {
										// TODO: handle read fault
										println!("@{:08X}::{:08X} READ FAULT RM HNS 0x{:08X}", cpu.S_base[PS], cpu.R[PC], addr);
										// for now
										cpu.running.store(false, Ordering::Relaxed);
									},
									Ok(x) => {
										cpu.R[rr_reg_d(iword0)] = (cpu.R[rr_reg_d(iword0)] & 0xFFFF0000) | (x as u32);
									},
								};
							} else {
								// TODO: handle segmentation fault
								println!("@{:08X}::{:08X} SEGMENTATION FAULT RM HNS 0x{:08X}", cpu.S_base[PS], cpu.R[PC], addr);
								// for now
								cpu.running.store(false, Ordering::Relaxed);
							}
						},
						
						0b01101000 => { // RM ST, store word
							let addr = cpu.gen_addr_rm(rm_seg_s(iword1), rr_reg_r(iword0), iword1);
							if cpu.access_check(rm_seg_s(iword1), addr, true, false) {
								match held_bus.write_w(addr, cpu.R[rr_reg_d(iword0)]) {
									Err(_) => {
										// TODO: handle write fault
										println!("@{:08X}::{:08X} WRITE FAULT RM ST 0x{:08X}", cpu.S_base[PS], cpu.R[PC], addr);
										// for now
										cpu.running.store(false, Ordering::Relaxed);
									},
									Ok(_) => { /* do nothing */ },
								};
							} else {
								// TODO: handle segmentation fault
								println!("@{:08X}::{:08X} SEGMENTATION FAULT RM ST 0x{:08X}", cpu.S_base[PS], cpu.R[PC], addr);
								// for now
								cpu.running.store(false, Ordering::Relaxed);
							}
						},
						0b01101001 => { // RM BST, store byte
							let addr = cpu.gen_addr_rm(rm_seg_s(iword1), rr_reg_r(iword0), iword1);
							if cpu.access_check(rm_seg_s(iword1), addr, true, false) {
								match held_bus.write_b(addr, (cpu.R[rr_reg_d(iword0)] & 0xFF) as u8) {
									Err(_) => {
										// TODO: handle write fault
										println!("@{:08X}::{:08X} WRITE FAULT RM BST 0x{:08X}", cpu.S_base[PS], cpu.R[PC], addr);
										// for now
										cpu.running.store(false, Ordering::Relaxed);
									},
									Ok(_) => { /* do nothing */ },
								};
							} else {
								// TODO: handle segmentation fault
								println!("@{:08X}::{:08X} SEGMENTATION FAULT RM BST 0x{:08X}", cpu.S_base[PS], cpu.R[PC], addr);
								// for now
								cpu.running.store(false, Ordering::Relaxed);
							}
						},
						0b01101010 => { // RM HST, store half
							let addr = cpu.gen_addr_rm(rm_seg_s(iword1), rr_reg_r(iword0), iword1);
							if cpu.access_check(rm_seg_s(iword1), addr, true, false) {
								match held_bus.write_h(addr, (cpu.R[rr_reg_d(iword0)] & 0xFFFF) as u16) {
									Err(_) => {
										// TODO: handle write fault
										println!("@{:08X}::{:08X} WRITE FAULT RM HST 0x{:08X}", cpu.S_base[PS], cpu.R[PC], addr);
										// for now
										cpu.running.store(false, Ordering::Relaxed);
									},
									Ok(_) => { /* do nothing */ },
								};
							} else {
								// TODO: handle segmentation fault
								println!("@{:08X}::{:08X} SEGMENTATION FAULT RM HST 0x{:08X}", cpu.S_base[PS], cpu.R[PC], addr);
								// for now
								cpu.running.store(false, Ordering::Relaxed);
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
				
				cpu.cycles = cpu.cycles.wrapping_add(1);
			}
			println!("@{:08X}::{:08X} CPU STOP - {} cycles", cpu.S_base[PS], cpu.R[PC], cpu.cycles);
		});
	}
}