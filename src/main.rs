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
	
	cpu.SDTR_base = 0xF00;
	cpu.SDTR_len = 0xFF;
	
	let mut bus3 = bus2.lock().unwrap();
	
	bus3.write_w(0,  0x10_00_10_61);     // 61 10 00 10: (RM)   LA      1,  0:       #16
	bus3.write_h(4,  0x11_0E);           // 0E 11      : (RR)   SQ      1,           #1
	bus3.write_h(6,  0x02_3E);           // 3E 12      : (SK)   IFS
	bus3.write_w(8,  0xF8_7F_FF_61);     // 61 FF FF FA: (RM)   B                    #-8
	
	bus3.write_h(12, 0x31_2B);		     // 2B 31      : (RR)   SSELHC  3,           #1
	bus3.write_h(14, 0x00_00);           // 00 00      : (RR)   NOP
	bus3.write_w(16, 0x00_30_E0_7F);     // 7F E0 30 00: (RM)   BL          3: 0
	bus3.write_w(20, 0x00_70_00_7F);     // FF FF FF FF: (ILL)  STOP
	
	// the CPU should reach an Application Fault at this point
	
	bus3.write_w(0xC00, 0xFFFFFFFF);
	
	bus3.write_w(0xF0C, 0x0000F000);
	bus3.write_w(0xF10, 0x00010000);
	bus3.write_b(0xF14, 0xFF);
	bus3.write_b(0xF15, 0xF0);
	
	bus3.write_w(0xF000, 0xFF_7F_20_61); // 61 20 FF FF: (RM)   LA      2, PS:       #-1
	bus3.write_w(0xF004, 0x00_60_0E_7F); // 7F 0E E0 00: (RM)   RTL
	
	drop(bus3);
	
	let mut running = Arc::clone(&cpu.running);
		
	let arc = Arc::new(Mutex::new(cpu));
	
	SeriesQ::run(Arc::clone(&arc));
	thread::sleep(time::Duration::from_millis(500));
	
	let mut x = 0;
	channel.in_channel(|bus: &mut bus::Bus| -> () {
		x = bus.read_w(0x0000F000).unwrap();
	});
	println!("DMA: Got 0x{:08X}", x);
	
	thread::sleep(time::Duration::from_millis(500));
	running.store(false, Ordering::Relaxed);
	thread::sleep(time::Duration::from_millis(1000));
	
	let c = arc.lock().unwrap();
	println!("R1 : 0x{:08X}", c.R[1]);
	println!("R2 : 0x{:08X}", c.R[2]);
	println!("R3 : 0x{:08X}", c.R[3]);
	println!("R4 : 0x{:08X}", c.R[4]);
	println!("R5 : 0x{:08X}", c.R[5]);
	println!("R6 : 0x{:08X}", c.R[6]);
	println!("R7 : 0x{:08X}", c.R[7]);
	println!("R8 : 0x{:08X}", c.R[8]);
	println!("R9 : 0x{:08X}", c.R[9]);
	println!("R10: 0x{:08X}", c.R[10]);
	println!("R11: 0x{:08X}", c.R[11]);
	println!("R12: 0x{:08X}", c.R[12]);
	println!("R13: 0x{:08X}", c.R[13]);
	println!("LR : 0x{:08X}", c.R[14]);
	println!("PC : 0x{:08X}", c.R[15]);
	println!("S3 : 0x{:08X}->0x{:08X}; 0x{:02X}, 0x{:02X}", c.S_base[3], c.S_limit[3], c.S_key[3], c.S_flags[3]);
}
