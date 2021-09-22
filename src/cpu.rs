use std::sync::{Arc, Mutex, Condvar};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::{thread, time};
use crate::bus::{Bus, Channel, Memory32, BusError};

pub const PC: usize = 15;
pub const LR: usize = 14;

pub const PS: usize = 7;
pub const LS: usize = 6;

pub const SUPERVISOR_ACCESS: i32 = -1;
pub const OUT_OF_BOUNDS: i32 = -2;
pub const ILLEGAL_INSTRUCTION: i32 = -3;
pub const SEGMENTATION_FAULT: i32 = -4;
pub const READ_FAULT: i32 = -5;
pub const WRITE_FAULT: i32 = -6;

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
	
	pub F: [u8; 16], // F0: PLGEVCSB; F8: .F__P__A (..., Fault Priority Level, Current Priority Level, Application State)
					 // F10, F11: Fault Instruction; F12-F15: Fault Address
	
	pub SDTR_base: u32,
	pub SDTR_len: u8,
	
	pub PEBA_base: u32,
	pub PLBA_base: u32,
	
	pub running: Arc<AtomicBool>,
	pub cycles: u64,
	
	pub bus: Arc<Mutex<Bus>>,
	pub channels: Vec<Channel<Bus>>,
	pub ipl: Vec<Arc<AtomicBool>>,
	pub icode: Vec<Arc<AtomicU8>>,
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
		let segment_check = (self.MPK.contains(&self.S_key[segment]) || &self.F[8] & 1 == 0)
			&& addr >= self.S_base[segment]
			&& addr < self.S_limit[segment];
		
		let read_allowed = (self.S_flags[segment] & 0b10000000 != 0);
		let write_allowed = (self.S_flags[segment] & 0b01000000 != 0);
		let exec_allowed = (self.S_flags[segment] & 0b00100000 != 0);
		
		if &self.F[8] & 1 != 0 { // if application state
			if write {
				segment_check && write_allowed
			} else if exec {
				segment_check && exec_allowed
			} else {
				segment_check && read_allowed
			}
		} else {
			segment_check
		}
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
		self.F[12] = (addr & 0xFF) as u8;
		self.F[13] = ((addr & 0xFF00) >> 8) as u8;
		self.F[14] = ((addr & 0xFF0000) >> 16) as u8;
		self.F[15] = ((addr & 0xFF000000) >> 24) as u8;
		self.app_fault(iword0, READ_FAULT as u32);
	}
	fn write_fault(&mut self, iword0: u16, addr: u32) {
		self.F[12] = (addr & 0xFF) as u8;
		self.F[13] = ((addr & 0xFF00) >> 8) as u8;
		self.F[14] = ((addr & 0xFF0000) >> 16) as u8;
		self.F[15] = ((addr & 0xFF000000) >> 24) as u8;
		self.app_fault(iword0, WRITE_FAULT as u32);
	}
	fn seg_fault(&mut self, iword0: u16, addr: u32) {
		self.F[12] = (addr & 0xFF) as u8;
		self.F[13] = ((addr & 0xFF00) >> 8) as u8;
		self.F[14] = ((addr & 0xFF0000) >> 16) as u8;
		self.F[15] = ((addr & 0xFF000000) >> 24) as u8;
		self.app_fault(iword0, SEGMENTATION_FAULT as u32);
	}
	fn app_fault(&mut self, iword0: u16, error_code: u32) {
		if self.F[8] & 1 == 0 {
			// we are in supervisor state
			self.sys_fault(iword0, error_code);
		} else {
			// TODO: priority level nonsense
			println!("@{:08X}::{:08X} 0x{:04X} APPLICATION FAULT 0x{:08X}", self.S_base[PS], self.R[PC], iword0, error_code);
			
			let new_pl = (self.F[8] & 0x70) >> 4;
			
			self.S_selector[PS] = (error_code & 0xFF) as u8;
			self.F[10] = (iword0 & 0xFF) as u8;
			self.F[11] = ((iword0 & 0xFF00) >> 8) as u8;
			self.running.store(false, Ordering::Relaxed);
			
			if (self.F[8] & 0xE) >> 1 == 7 {
				self.running.store(false, Ordering::Relaxed);
			} else {
				self.ipl[new_pl as usize].store(true, Ordering::Relaxed);
				self.icode[new_pl as usize].store((error_code & 0xFF) as u8, Ordering::Relaxed);
			}
		}
	}
	fn sys_fault(&mut self, iword0: u16, error_code: u32) {
		println!("@{:08X}::{:08X} 0x{:04X} SYSTEM FAULT 0x{:08X}", self.S_base[PS], self.R[PC], iword0, error_code);
		self.F[10] = (iword0 & 0xFF) as u8;
		self.F[11] = ((iword0 & 0xFF00) >> 8) as u8;
		
		// we should never get here; escalate to max pl or halt
		if (self.F[8] & 0xE) >> 1 == 7 {
			self.running.store(false, Ordering::Relaxed);
		} else {
			self.ipl[7].store(true, Ordering::Relaxed);
			self.icode[7].store((error_code & 0xFF) as u8, Ordering::Relaxed);
		}
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
					  0x00,
					  0xFF,
					  0xFF,
					  0xFF,
					  0xFF,
					  0xFF,
					  0xFF,
					  0xFF,
					  0xF0],
			
			MPK: [0xFF; 16],
			
			F: [0xFE; 16],
			
			SDTR_base: 0,
			SDTR_len: 0,
			
			PEBA_base: 0,
			PLBA_base: 0,
			
			running: Arc::new(AtomicBool::new(false)),
			cycles: 0,
			
			bus: bus,
			channels: Vec::new(),
			ipl: Vec::new(),
			icode: Vec::new()
		};
		
		for _ in 0..16 {
			result.channels.push(Channel::new(&result.bus));
		}
		for _ in 0..8 {
			result.ipl.push(Arc::new(AtomicBool::new(false)));
		}
		for _ in 0..8 {
			result.icode.push(Arc::new(AtomicU8::new(0)));
		}
		
		result
	}
	
	fn pl_set(&mut self, pl: u8, ssr7: u8, bus: &mut Bus) {
		
		let new_priority = pl & 0x7;
		
		let old_ps_base = self.S_base[PS];
		let old_ps_limit = self.S_limit[PS];
		
		let old_ps_key = self.S_key[PS];
		let old_ps_flags = self.S_flags[PS];
		let old_sr8 = self.F[8];
		let old_ps_selector = self.S_selector[PS];
		let old_lba2 = (old_ps_key as u32) | (old_ps_flags as u32) << 8 | (old_sr8 as u32) << 16 | (old_ps_selector as u32) << 24;
		
		let old_pc = self.R[PC];
		
		// write out PLBA for target priority level
		
		let mut error = false;
		loop {
			let link_block_offset = self.PLBA_base + 16 * new_priority as u32;
			
			match bus.write_w(link_block_offset, old_ps_base) {
				Err(_) => {
					self.write_fault(0xFFFF, link_block_offset);
					error = true;
					break;
				},
				Ok(_) => { /* do nothing */ },
			};
			
			match bus.write_w(link_block_offset + 4, old_ps_limit) {
				Err(_) => {
					self.write_fault(0xFFFF, link_block_offset + 4);
					error = true;
					break;
				},
				Ok(_) => { /* do nothing */ },
			};
			
			match bus.write_w(link_block_offset + 8, old_lba2) {
				Err(_) => {
					self.write_fault(0xFFFF, link_block_offset + 8);
					error = true;
					break;
				},
				Ok(_) => { /* do nothing */ },
			};
			
			match bus.write_w(link_block_offset + 12, old_pc) {
				Err(_) => {
					self.write_fault(0xFFFF, link_block_offset + 12);
					error = true;
					break;
				},
				Ok(_) => { /* do nothing */ },
			};
			
			break;
		}
		
		if error {
			return;
		}
		
		// read in PEBA for target priority level
		
		loop {
			let entry_block_offset = self.PEBA_base + 16 * new_priority as u32;
			
			match bus.read_w(entry_block_offset) {
				Err(_) => {
					self.read_fault(0xFFFF, entry_block_offset);
					error = true;
					break;
				},
				Ok(x) => { self.S_base[PS] = x; },
			};
			
			match bus.read_w(entry_block_offset + 4) {
				Err(_) => {
					self.read_fault(0xFFFF, entry_block_offset + 4);
					error = true;
					break;
				},
				Ok(x) => { self.S_limit[PS] = x; },
			};
			
			match bus.read_w(entry_block_offset + 8) {
				Err(_) => {
					self.read_fault(0xFFFF, entry_block_offset + 8);
					error = true;
					break;
				},
				Ok(x) => {
					self.S_key[PS] = (x & 0xFF) as u8;
					self.S_flags[PS] = ((x & 0xFF00) >> 8) as u8;
					self.F[8] = ((x & 0xFF0000) >> 16) as u8;
					self.F[8] &= !(0xE);
					self.F[8] |= new_priority << 1;
					self.S_selector[PS] = ssr7;
				},
			};
			
			match bus.read_w(entry_block_offset + 12) {
				Err(_) => {
					self.read_fault(0xFFFF, entry_block_offset + 12);
					error = true;
					break;
				},
				Ok(x) => { self.R[PC] = x; },
			};
			
			break;
		}
	}

	fn pl_esc(&mut self, pl: u8, ssr7: u8, bus: &mut Bus) -> bool {
		let new_priority = pl & 0x7;
		let old_priority = (self.F[8] & 0xE) >> 1;
		
		if new_priority > old_priority {
			self.pl_set(new_priority, ssr7, bus);
			true
		} else {
			false
		}
	}
	
	fn pl_retn(&mut self, bus: &mut Bus) {
		// restore old priority level		
		let mut error = false;
		loop {
			let link_block_offset = self.PLBA_base + 16 * ((self.F[8] & 0xE) >> 1) as u32;
			
			match bus.read_w(link_block_offset) {
				Err(_) => {
					self.read_fault(0xFFFF, link_block_offset);
					error = true;
					break;
				},
				Ok(x) => { self.S_base[PS] = x; },
			};
			
			match bus.read_w(link_block_offset + 4) {
				Err(_) => {
					self.read_fault(0xFFFF, link_block_offset + 4);
					error = true;
					break;
				},
				Ok(x) => { self.S_limit[PS] = x; },
			};
			
			match bus.read_w(link_block_offset + 8) {
				Err(_) => {
					self.read_fault(0xFFFF, link_block_offset + 8);
					error = true;
					break;
				},
				Ok(x) => {
					self.S_key[PS] = (x & 0xFF) as u8;
					self.S_flags[PS] = ((x & 0xFF00) >> 8) as u8;
					self.F[8] = ((x & 0xFF0000) >> 16) as u8;
					self.S_selector[PS] = ((x & 0xFF000000) >> 24) as u8;
				},
			};
			
			match bus.read_w(link_block_offset + 12) {
				Err(_) => {
					self.read_fault(0xFFFF, link_block_offset + 12);
					error = true;
					break;
				},
				Ok(x) => { self.R[PC] = x; },
			};
			
			break;
		}
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
				
				// cpu.pl_set(3, &mut held_bus);
				
				
				// instruction fetch
				let mut iword0: u16 = 0;
				let mut iword1: u16 = 0;
				let mut ifetch = true;
				
				let addr = cpu.R[PC].wrapping_add(cpu.S_base[PS]);
				if cpu.access_check(PS, addr, false, true) {
					match held_bus.read_h_big(cpu.R[PC].wrapping_add(cpu.S_base[PS])) {
						Err(_) => {
							ifetch = false;
							// for now
							cpu.read_fault(0xFFFF, addr);
						},
						Ok(x) => { iword0 = x; cpu.R[PC] = cpu.R[PC].wrapping_add(2); },
					};
				} else {
					ifetch = false;
					// for now
					cpu.seg_fault(0xFFFF, addr);
				}
				
				// TODO: fetch rest of instruction
				
				if ifetch && cpu.increment(iword0) >= 4 {
					let addr = cpu.R[PC].wrapping_add(cpu.S_base[PS]);
					if cpu.access_check(PS, addr, false, true) {
						match held_bus.read_h_big(cpu.R[PC].wrapping_add(cpu.S_base[PS])) {
							Err(_) => {
								ifetch = false;
								// for now
								cpu.read_fault(0xFFFF, addr);
							},
							Ok(x) => { iword1 = x; cpu.R[PC] = cpu.R[PC].wrapping_add(2); },
						};
					} else {
						ifetch = false;
						// for now
						cpu.seg_fault(0xFFFF, addr);
					}
				}
				
				if ifetch && !skip {
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
						0b00011110 => { // SLQL, long quick shift left
							let (x, flags) = alu_shl(cpu.R[rr_reg_d(iword0)], rr_reg_r(iword0) as u32 + 16, cpu.F[0]);
							cpu.R[rr_reg_d(iword0)] = x;
							cpu.F[0] = flags;
						},
						0b00011111 => { // SRQL, long quick shift right
							let (x, flags) = alu_shr(cpu.R[rr_reg_d(iword0)], rr_reg_r(iword0) as u32 + 16, cpu.F[0]);
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
							
							let mut ok = true;
							
							// set PEBA
							let addr = cpu.SDTR_base;
							match held_bus.read_w(addr) {
								Err(_) => {
									cpu.read_fault(iword0, addr);
									ok = false;
								},
								Ok(x) => { cpu.PEBA_base = x; },
							};
							
							// set PLBA
							if ok {
								let addr = cpu.SDTR_base + 12;
								match held_bus.read_w(addr) {
									Err(_) => {
										cpu.read_fault(iword0, addr);
										ok = false;
									},
									Ok(x) => { cpu.PLBA_base = x; },
								};
							}
						},
						
						0b00100110 => { // LSEL, load segment selector
							cpu.R[rr_reg_d(iword0)] = cpu.S_selector[rr_reg_r(iword0)] as u32;
						}
						0b00100111 => { // SSEL, set segment selector
							if (cpu.F[8] & 0b00000001 != 0 && rr_reg_d(iword0) >= 8) {
								cpu.app_fault(iword0, SUPERVISOR_ACCESS as u32);
							} else if ((cpu.R[rr_reg_r(iword0)] & 0xFF) as u8) > cpu.SDTR_len {
								cpu.app_fault(iword0, OUT_OF_BOUNDS as u32);
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
							if (cpu.F[8] & 0b00000001 != 0 && rr_reg_d(iword0) >= 8) {
								cpu.app_fault(iword0, SUPERVISOR_ACCESS as u32);
							} else if ((rr_reg_r(iword0) & 0xFF) as u8) > cpu.SDTR_len {
								cpu.app_fault(iword0, OUT_OF_BOUNDS as u32);
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
							// handle illegal instruction
							cpu.app_fault(0xFFFF, ILLEGAL_INSTRUCTION as u32);
						},
					};
				} else if skip {
					skip = false;
				}
				
				// service interrupts
				
				let mut new_pl = 0;
				for (index, state) in cpu.ipl.iter().enumerate() {
					if state.load(Ordering::Relaxed) && index > new_pl {
						new_pl = index;
					}
				}
				let new_code = cpu.icode[new_pl].load(Ordering::Relaxed);
				cpu.pl_esc((new_pl & 0xFF) as u8, new_code, &mut held_bus);
				
				// service DMA
				
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
