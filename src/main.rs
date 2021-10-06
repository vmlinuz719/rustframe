use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::{thread, time};
mod bus;
mod cpu;
use crate::bus::Memory32;
use crate::cpu::{SeriesQ, SQAddr};

fn main() {
	let mem = Arc::new(Mutex::new(vec![0 as u8; 65536]));
	let mem_clone = Arc::clone(&mem);
	let mut b = bus::Bus::new();
	b.attach(0, 65535, mem_clone);
	
	let bus = Arc::new(Mutex::new(b));
	let bus2 = Arc::clone(&bus);
	
	let mut cpu = cpu::SeriesQ::new(bus);
	let channel = bus::Channel::clone(&cpu.channels[0]);
	
	let mut bus3 = bus2.lock().unwrap();
	
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
	bus3.write_w(0xF34, 0x00006FFF);
	bus3.write_b(0xF38, 0x0E);
	bus3.write_b(0xF39, 0xE0);
	
	// service dispatch table segment
	bus3.write_w(0xF3C, 0x00000000);
	bus3.write_w(0xF40, 0x00000200);
	bus3.write_b(0xF44, 0xEB);
	bus3.write_b(0xF45, 0x01);
	
	// user exit trampoline
	bus3.write_w(0xC000, 0x00002000);
	bus3.write_w(0xC004, 0x00002FFF);
	bus3.write_b(0xC008, 0xFF);
	bus3.write_b(0xC009, 0x00);
	bus3.write_b(0xC00A, 0x7E);
	bus3.write_b(0xC00B, 0x01);
	bus3.write_w(0xC00C, 0x00000000);
	
	// test LBA entry
	bus3.write_w(0xC070, 0x00004000);
	bus3.write_w(0xC074, 0x00006FFF);
	bus3.write_b(0xC078, 0x0E);
	bus3.write_b(0xC079, 0xE0);
	bus3.write_b(0xC07A, 0x71);
	bus3.write_b(0xC07B, 0x04);
	bus3.write_w(0xC07C, 0x00000000);
	
	// test EBA entry
	bus3.write_w(0xC170, 0x00002000);
	bus3.write_w(0xC174, 0x00002FFF);
	bus3.write_b(0xC178, 0xFF);
	bus3.write_b(0xC179, 0x00);
	bus3.write_b(0xC17A, 0x7E);
	bus3.write_b(0xC17B, 0x00);
	bus3.write_w(0xC17C, 0x00000100);
	
	bus3.write_h(0x1000, 0x1F_01);	// LFI 1, 15
	bus3.write_h(0x1002, 0x17_1C);  // SLFI 1, 7
	bus3.write_h(0x1004, 0x26_01);	// LFI 2, 6
	bus3.write_h(0x1006, 0x12_25);	// SSBA 1, 2
	bus3.write_h(0x1008, 0x72_2B);	// SLSFI 7, 2
	bus3.write_h(0x100A, 0x03_2B);	// SLSFI 0, 3
	bus3.write_h(0x100C, 0x3E_01);	// LFI 3, 14
	bus3.write_h(0x100E, 0x03_29);	// SMPK 0, 3
	bus3.write_h(0x1010, 0x00_30);	// PLR
	
	bus3.write_w(0x2000, 0xFE_00);			// RT						(MV 15, 14)
	
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
		
	bus3.write_w(0x4000, 0x23_01_50_61);	// LA 5, 0: 0, X'123'
	bus3.write_h(0x4004, 0x54_1E);			// SLFIL 5, 4
	bus3.write_h(0x4006, 0x00_00);			// NOP
	bus3.write_w(0x4008, 0x56_04_60_61);	// LA 6, 0: 0, X'456'
	bus3.write_h(0x400C, 0x67_1C);			// SLFI 6, 7
	bus3.write_h(0x400E, 0x56_11);			// O 5, 6
	bus3.write_w(0x4010, 0x78_00_60_41);	// LA 6, 0: 0, 0, X'78'
	bus3.write_h(0x4014, 0x56_06);			// BIN 5, 6
	bus3.write_h(0x4016, 0x60_00);			// MV 6, 0
	bus3.write_h(0x4018, 0x11_31);			// SVC X'11'
	bus3.write_h(0x401A, 0xFF_FF);
	
	drop(bus3);
	
	let mut running = Arc::clone(&cpu.running);
		
	let arc = Arc::new(Mutex::new(cpu));
	
	SeriesQ::run(Arc::clone(&arc));
	thread::sleep(time::Duration::from_millis(1000));
	
	// let mut x = 0;
	// channel.in_channel(|bus: &mut bus::Bus| -> () {
		// x = bus.read_w(0x0000F000).unwrap();
	// });
	// println!("DMA: Got 0x{:08X}", x);
	running.store(false, Ordering::Relaxed);
	thread::sleep(time::Duration::from_millis(1000));
	
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
	println!("SR8  : 0b{:08b}", c.F[8]);
	
	for x in 0..15 {
		println!("SSR{:<2}: 0x{:02X} (0x{:08X}->0x{:08X}; 0x{:02X}, 0x{:02X})", x, c.S_selector[x], c.S_base[x], c.S_limit[x], c.S_key[x], c.S_flags[x]);
	}
}
