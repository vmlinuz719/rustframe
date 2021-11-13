use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicU8, AtomicU16, AtomicBool, Ordering};
use std::{thread, time};
mod bus;
mod cpu;
use crate::bus::{Memory32, BusError};
use crate::cpu::{SeriesQ, SQAddr};

extern crate encoding;
use encoding::{Encoding, EncoderTrap, DecoderTrap};
use encoding::all::ISO_8859_1;

struct LP1204 {
	pub ipl: Arc<AtomicBool>,
	pub icode: Arc<AtomicU8>,
	
	pub buffer: Arc<Mutex<Vec<u8>>>,
	
	pub running: Arc<AtomicBool>
}

impl LP1204 {
	pub fn new(ipl_line: Arc<AtomicBool>, ipl_code: Arc<AtomicU8>) -> LP1204 {
		let buf = Arc::new(Mutex::new(vec![0 as u8; 256]));
		
		LP1204 {
			ipl: ipl_line,
			icode: ipl_code,
			buffer: buf,
			running: Arc::new(AtomicBool::new(false))
		}
	}
	
	pub fn run(prt: Arc<Mutex<LP1204>>) {
		thread::spawn(move || {
			let prt = prt.lock().unwrap();
			
			prt.running.store(true, Ordering::Relaxed);
			
			while prt.running.load(Ordering::Relaxed) {
				let mut buf = prt.buffer.lock().unwrap();
				let mut exec: u8 = 0;
				
				match buf.read_b(148) {
					Err(e) => {
						println!("FATAL PRINTER ERROR");
						break;
					},
					Ok(x) => { exec = x; },
				};
				
				if exec != 0 {
					if buf[144] == 0 { // Print Buffer
						let cleaned: Vec<u8> = buf[0..144].iter().map(|&x| {
							match x {
								0x00..=0x1F => 0x20,
								0x7F..=0xA0 => 0x20,
								0xAD => 0x2D,
								_ => x
							}
						}).collect();
						
						let line = ISO_8859_1.decode(&cleaned, DecoderTrap::Replace).unwrap();
						
						println!("{}", line);
						thread::sleep(time::Duration::from_millis(90));
					}
					
					match buf.write_b(148, 0) {
					Err(e) => {
						println!("FATAL PRINTER ERROR");
						break;
					},
					Ok(_) => { },
				};
				}
			}
		});
	}
}

struct Port {
	pub tx: AtomicU16,
	pub rx: AtomicU16,
	pub lines: AtomicU8, // [3210IEAR] - Device Specific Lines, Inbound, Error, Acknowledge, Ready
	pub imask: AtomicU8,
	pub strobe: AtomicBool,
	
	pub ipl: Arc<AtomicBool>
}

impl Port {
	pub fn new(ipl_line: Arc<AtomicBool>) -> Port {
		Port {
			tx: AtomicU16::new(0),
			rx: AtomicU16::new(0),
			lines: AtomicU8::new(0),
			imask: AtomicU8::new(0),
			strobe: AtomicBool::new(false),
			
			ipl: ipl_line
		}
	}
	
	
	// peripheral side
	pub fn send(&self, data: u16) {
		self.rx.store(data, Ordering::SeqCst);
	}
	
	pub fn recv(&self) -> u16 {
		if self.strobe.load(Ordering::SeqCst) {
			self.strobe.store(false, Ordering::SeqCst);
			self.tx.load(Ordering::SeqCst)
		} else {
			0
		}
	}
	
	pub fn flag(&self, data: u8) {
		self.lines.store(data, Ordering::SeqCst);
		if data & self.imask.load(Ordering::SeqCst) != 0 {
			self.ipl.store(true, Ordering::SeqCst);
		}
	}
	
	// bus side
	pub fn write(&self, data: u16) {
		self.tx.store(data, Ordering::SeqCst);
		self.strobe.store(true, Ordering::SeqCst);
	}
	
	pub fn read(&self) -> u16 {
		self.ipl.store(false, Ordering::SeqCst);
		self.rx.load(Ordering::SeqCst)
	}
	
}

impl Memory32<u32, BusError> for Port {
	fn read_b(&self, addr: u32) -> Result<u8, BusError> {
		match addr {
			0 => Ok((self.read() & 0xFF) as u8),
			1 => Ok(((self.read() & 0xFF00) >> 8) as u8),
			2 => Ok({
				let x = self.lines.load(Ordering::SeqCst);
				self.lines.store(0, Ordering::SeqCst);
				x
				}),
			3 => Ok(self.imask.load(Ordering::SeqCst)),
			_ => Err(BusError::InvalidAddress)
		}
	}
	fn read_h(&self, addr: u32) -> Result<u16, BusError> {
		match addr {
			0 => Ok(self.read()),
			_ => Err(BusError::InvalidAddress)
		}
	}
	fn read_h_big(&self, addr: u32) -> Result<u16, BusError> {
		Err(BusError::InvalidAddress)
	}
	fn read_w(&self, addr: u32) -> Result<u32, BusError> {
		Err(BusError::InvalidAddress)
	}
	
	fn write_b(&mut self, addr: u32, data: u8) -> Result<(), BusError> {
		match addr {
			// 2 => Ok(self.lines.store(data, Ordering::SeqCst)),
			3 => Ok(self.imask.store(data, Ordering::SeqCst)),
			_ => Err(BusError::InvalidAddress)
		}
	}
	fn write_h(&mut self, addr: u32, data: u16) -> Result<(), BusError> {
		match addr {
			0 => {
				self.strobe.store(true, Ordering::SeqCst);
				Ok(self.tx.store(data, Ordering::SeqCst))
			},
			_ => Err(BusError::InvalidAddress)
		}
	}
	fn write_w(&mut self, addr: u32, data: u32) -> Result<(), BusError> {
		Err(BusError::InvalidAddress)
	}
}

fn main() {
	let mem = Arc::new(Mutex::new(vec![0 as u8; 65536]));
	let mem_clone = Arc::clone(&mem);
	let mut b = bus::Bus::new();
	b.attach(0, 65536, mem_clone);
	
	let bus = Arc::new(Mutex::new(b));
	let bus2 = Arc::clone(&bus);
	
	let mut cpu = cpu::SeriesQ::new(bus);
	let channel = bus::Channel::clone(&cpu.channels[0]);
	
	let prt = LP1204::new( Arc::clone(&cpu.ipl[4]), Arc::clone(&cpu.icode[4]) );
	let prt_buf = Arc::clone(&prt.buffer);
	bus2.lock().unwrap().attach(65536, 256, prt_buf);
	
	let prt_runnable = Arc::new(Mutex::new(prt));
	
	let dataport = Arc::new(Mutex::new(Port::new(Arc::clone(&cpu.ipl[6]))));
	let dp2 = Arc::clone(&dataport);
	let dp3 = Arc::clone(&dataport);
	bus2.lock().unwrap().attach(0x20000, 4, dp2);
	
	///*
	let mut bus3 = bus2.lock().unwrap();
	
	{
	
	// set EBA
	bus3.write_w(0xF00, 0x0000C100);
	bus3.write_w(0xF04, 0x0000C180);
	bus3.write_b(0xF08, 0xEB);
	bus3.write_b(0xF09, 0x01);
	
	// set LBA
	bus3.write_w(0xF0C, 0x0000C000);
	bus3.write_w(0xF10, 0x0000C080);
	bus3.write_b(0xF14, 0xEB);
	bus3.write_b(0xF15, 0x01);
	
	// flat memory model with code and data segment
	bus3.write_w(0xF18, 0x00000000);
	bus3.write_w(0xF1C, 0x00010000);
	bus3.write_b(0xF20, 0xEB);
	bus3.write_b(0xF21, 0x00);
	
	bus3.write_w(0xF24, 0x00000000);
	bus3.write_w(0xF28, 0x00010000);
	bus3.write_b(0xF2C, 0xEB);
	bus3.write_b(0xF2D, 0x01);
		
	// user program segment
	bus3.write_w(0xF30, 0x00004000);
	bus3.write_w(0xF34, 0x00007000);
	bus3.write_b(0xF38, 0x0E);
	bus3.write_b(0xF39, 0xE0);
	
	// service dispatch table segment
	bus3.write_w(0xF3C, 0x00000000);
	bus3.write_w(0xF40, 0x00000200);
	bus3.write_b(0xF44, 0xEB);
	bus3.write_b(0xF45, 0x01);
	
	// 1204 line printer
	bus3.write_w(0xF48, 0x00010000);
	bus3.write_w(0xF4C, 0x00010098);
	bus3.write_b(0xF50, 0x0E);
	bus3.write_b(0xF51, 0xE0);
	
	// 2200 data port interface
	bus3.write_w(0xF54, 0x00020000);
	bus3.write_w(0xF58, 0x00020004);
	bus3.write_b(0xF5C, 0x0E);
	bus3.write_b(0xF5D, 0xE0);
	
	// user exit trampoline
	bus3.write_w(0xC000, 0x00002000);
	bus3.write_w(0xC004, 0x00003000);
	bus3.write_b(0xC008, 0xFF);
	bus3.write_b(0xC009, 0x00);
	bus3.write_b(0xC00A, 0x7E);
	bus3.write_b(0xC00B, 0x01);
	bus3.write_w(0xC00C, 0x00000000);
	
	// test LBA entry
	bus3.write_w(0xC070, 0x00004000);
	bus3.write_w(0xC074, 0x00007000);
	bus3.write_b(0xC078, 0x0E);
	bus3.write_b(0xC079, 0xE0);
	bus3.write_b(0xC07A, 0x71);
	bus3.write_b(0xC07B, 0x04);
	bus3.write_w(0xC07C, 0x00000000);
	
	// test EBA entry
	bus3.write_w(0xC170, 0x00002000);
	bus3.write_w(0xC174, 0x00003000);
	bus3.write_b(0xC178, 0xFF);
	bus3.write_b(0xC179, 0x00);
	bus3.write_b(0xC17A, 0x7E);
	bus3.write_b(0xC17B, 0x00);
	bus3.write_w(0xC17C, 0x00000100);
	
	bus3.write_h(0x1000, 0x1F_01);	// LFI 1, 15
	bus3.write_h(0x1002, 0x17_1C);  // SLFI 1, 7
	bus3.write_h(0x1004, 0x27_01);	// LFI 2, 6
	bus3.write_h(0x1006, 0x12_25);	// SSBA 1, 2
	bus3.write_h(0x1008, 0x72_2B);	// SLSFI 7, 2
	bus3.write_h(0x100A, 0x03_2B);	// SLSFI 0, 3
	bus3.write_h(0x100C, 0xB6_2B);	// SLSFI 11, 6
	bus3.write_h(0x100E, 0xC7_2B);	// SLSFI 11, 7
	bus3.write_h(0x1010, 0x3E_01);	// LFI 3, 14
	bus3.write_h(0x1012, 0x03_29);	// SMPK 0, 3
	bus3.write_h(0x1014, 0x00_30);	// PLR
	
	bus3.write_w(0x2000, 0xFF_FF);			// HLT
	
	bus3.write_w(0x2100, 0xFC_70_1F_68);	// ST 1, 7, 15, +@R1SAVE
	bus3.write_w(0x2104, 0xFC_70_2F_68);	// ST 2, 7, 15, +@R2SAVE
	bus3.write_w(0x2108, 0xFC_70_3F_68);	// ST 3, 7, 15, +@R3SAVE
	bus3.write_w(0x210C, 0xFC_70_EF_68);	// ST 14, 7, 15, +@R4SAVE
	
	bus3.write_h(0x2110, 0x17_26);			// LSS 1, 7
	bus3.write_h(0x2112, 0x10_1C);			// SLFI 1, 0
	bus3.write_h(0x2114, 0x85_2B);			// SLSFI 8, 5
	bus3.write_h(0x2116, 0x00_00);			// NOP
	bus3.write_w(0x2118, 0x00_80_11_63);	// HTR 1, 8, 1
	bus3.write_w(0x211C, 0x02_70_EF_61);	// SBALR 1					(LA 14, 7, 15, X'2')
	bus3.write_h(0x2120, 0xF1_00);			// 							(MV 15, 1)
	
	bus3.write_h(0x2122, 0x00_00);			// NOP
	bus3.write_w(0x2124, 0xD8_70_1F_60);	// L 1, 7, 15, +@R1SAVE
	bus3.write_w(0x2128, 0xD8_70_2F_60);	// L 2, 7, 15, +@R2SAVE
	bus3.write_w(0x212C, 0xD8_70_3F_60);	// L 3, 7, 15, +@R3SAVE
	bus3.write_w(0x2130, 0xD8_70_EF_60);	// L 14, 7, 15, +@R4SAVE
	bus3.write_h(0x2134, 0x00_30);			// PLR
	
	bus3.write_w(0x2200, 0); // 0x2200: R1SAVE
	bus3.write_w(0x2204, 0); // 0x2204: R2SAVE
	bus3.write_w(0x2208, 0); // 0x2208: R3SAVE
	bus3.write_w(0x220C, 0); // 0x220C: LKSAVE
		
	bus3.write_w(0x4000, 0x00_71_10_61);	// LA 1, 7: 0, X'100'
	bus3.write_h(0x4004, 0x20_00);			// MV 2, 0
	bus3.write_h(0x4006, 0x00_00);			// NOP
	bus3.write_w(0x4008, 0x90_00_30_61);	// LA 3, 0: 0, X'90'
	
	bus3.write_w(0x400C, 0x00_72_41_42);	// BTR 4, 7: 1, 2
	
	bus3.write_h(0x4010, 0x23_20);			// C 2, 3
	bus3.write_h(0x4012, 0x10_3E);			// IFEQ
	bus3.write_w(0x4014, 0x18_70_FF_61);	// LA 15, 7: 15, +@PRINT
	
	bus3.write_h(0x4018, 0x40_20);			// C 4, 0
	bus3.write_h(0x401A, 0x10_3E);			// IFEQ
	bus3.write_w(0x401C, 0x10_70_FF_61);	// LA 15, 7: 15, +@PRINT
	
	bus3.write_w(0x4020, 0x00_B0_42_69);	// BST 4, 11: 2
	bus3.write_h(0x4024, 0x21_0C);			// AFI 2, 1
	bus3.write_h(0x4026, 0x00_00);			// NOP
	bus3.write_w(0x4028, 0x00_C0_40_6A);	// HST 4, 12: 0
	bus3.write_w(0x402C, 0xDC_7F_FF_61);	// LA 15, 7: 15, X'100'
	
	// PRINT:
	bus3.write_h(0x4030, 0x11_01);			// LFI 1, 1
	bus3.write_h(0x4032, 0x00_00);			// NOP
	bus3.write_w(0x4034, 0x94_B0_10_69);	// BST 1, 11: 0, X'94'
	bus3.write_h(0x4038, 0xFF_FF);			// NOP
	
	
	
	let string = ISO_8859_1.encode("0123456789 PORT TEST\0", EncoderTrap::Strict).unwrap();
	for (i, c) in string.iter().enumerate() {
		bus3.write_b(0x4100 + (i as u32), *c);
	}
	
	drop(bus3);
	
	}
	//*/
	
	
	
	let mut running = Arc::clone(&cpu.running);
		
	let arc = Arc::new(Mutex::new(cpu));
	
	thread::spawn(move || {
		let port = dp3.lock().unwrap();
		port.flag(0b00000001);
		drop(port);
		
		loop {
			// wait for port data
			loop {
				let port = dp3.lock().unwrap();
				if port.strobe.load(Ordering::SeqCst) {
					break;
				}
			}
			let port = dp3.lock().unwrap();
			println!("Got data {:04X}", port.recv());
			port.flag(0b00000011);
		}
	});
	
	SeriesQ::run(Arc::clone(&arc));
	LP1204::run(prt_runnable);
	thread::sleep(time::Duration::from_millis(2000));
	
	// let mut x = 0;
	// channel.in_channel(|bus: &mut bus::Bus| -> () {
		// x = bus.read_w(0x0000F000).unwrap();
	// });
	// println!("DMA: Got 0x{:08X}", x);
	running.store(false, Ordering::Relaxed);
	thread::sleep(time::Duration::from_millis(50));
	
	let c = arc.lock().unwrap();
	println!("R1   : 0x{:08X}", c.R[1]);
	println!("R2   : 0x{:08X}", c.R[2]);
	println!("R3   : 0x{:08X}", c.R[3]);
	println!("R4   : 0x{:08X}", c.R[4]);
	println!("R5   : 0x{:08X}", c.R[5]);
	println!("R6   : 0x{:08X}", c.R[6]);
	println!("R7   : 0x{:08X}", c.R[7]);
	println!("R8   : 0x{:08X}", c.R[8]);
	println!("R9   : 0x{:08X}", c.R[9]);
	println!("R10  : 0x{:08X}", c.R[10]);
	println!("R11  : 0x{:08X}", c.R[11]);
	println!("R12  : 0x{:08X}", c.R[12]);
	println!("R13  : 0x{:08X}", c.R[13]);
	println!("LR   : 0x{:08X}", c.R[14]);
	println!("PC   : 0x{:08X}", c.R[15]);
	
	println!("SR0  : 0b{:08b}", c.F[0]);
	println!("SR8  : 0b{:08b}", c.F[8]);
	
	for x in 0..15 {
		println!("SSR{:<2}: 0x{:02X} (0x{:08X}->0x{:08X}; 0x{:02X}, 0x{:02X})", x, c.S_selector[x], c.S_base[x], c.S_limit[x], c.S_key[x], c.S_flags[x]);
	}
}