use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::{thread, time};
mod bus;
mod cpu;
use crate::bus::Memory32;
use crate::cpu::{SeriesQ, SQAddr};

fn main() {
	/* let mem = Arc::new(Mutex::new(vec![0 as u8; 8192]));
	let mem2 = Arc::new(Mutex::new(vec![0 as u8; 8192]));
	let mut x = mem2.lock().unwrap();
	x.write_w(0, 8192);
	drop(x);
	
	let mem_clone = Arc::clone(&mem);
	let mem2_clone = Arc::clone(&mem2);
	
	let mut b = bus::Bus::new();
	b.attach(0, 8192, mem_clone);
	b.attach(8192, 8192, mem2_clone);
	
	let bus = Arc::new(Mutex::new(b));
	let channels = [bus::Channel::new(&bus), bus::Channel::new(&bus),
					bus::Channel::new(&bus), bus::Channel::new(&bus),
					bus::Channel::new(&bus), bus::Channel::new(&bus),
					bus::Channel::new(&bus), bus::Channel::new(&bus)];
	let ch0 = bus::Channel::clone(&channels[0]);
	let ch1 = bus::Channel::clone(&channels[1]);
	
	// simulate device on channel 0
	thread::spawn(move || {
		loop {
			thread::sleep(time::Duration::from_millis(1000));
			ch0.in_channel(|bus| {
				bus.read_b(0xDEADBEEF);
			});
			println!("Bus call successful");
		}
	});
	
	// simulate device on channel 1
	thread::spawn(move || {
		loop {
			thread::sleep(time::Duration::from_millis(2000));
			ch1.in_channel(|bus| {
				bus.read_b(0x0C07FEFE);
			});
			println!("Bus call successful");
		}
	});
	
	// simulate bus host
	let mut held_bus = bus.lock().unwrap();
	let mut cycles = 0;
	
	loop {
		cycles += 1;
		let x = held_bus.read_w(0).unwrap();
		let y = held_bus.read_w(8192).unwrap();
		
		for n in 0..8 {
			if channels[n].check_pending() {
				println!("CPU: Got BRQ on channel {0}, asserting BGR - \
					BEGIN DMA after {1} CPU cycles ({2}, {3})", n, cycles, x, y);
				cycles = 
				
				drop(held_bus);
				channels[n].open();
				held_bus = bus.lock().unwrap();
			}
		}
	} */
	
	let mem = Arc::new(Mutex::new(vec![0 as u8; 8192]));
	let mem_clone = Arc::clone(&mem);
	let mut b = bus::Bus::new();
	b.attach(0, 8192, mem_clone);
	
	let mut cpu = cpu::SeriesQ::new();
	
	cpu.SDTR_base = 0xF00;
	cpu.SDTR_len = 0xFF;
	
	b.write_w(0,  0x10_00_10_61); // 61 10 00 10: (RM)   LA    1,  0: 0,    #16
	b.write_h(4,  0x11_0E);       // 0E 11      : (RR)   SQ    1,           #1
	b.write_h(6,  0x02_3E);       // 3E 12      : (SK)   IFS
	b.write_w(8,  0xF8_FF_FF_61); // 61 FF FF FA: (RM)   LA   PC, PS:PC,    #-8
	
	b.write_w(12, 0x10_00_20_61); // 61 20 00 10: (RM)   LA    2,  0: 0,    #16
	b.write_h(16, 0x21_1C);       // 1C 21      : (RR)   SLQ   2,           #1
	b.write_h(18, 0x30_22);       // 22 30      : (RR)   LF    3,     0
	
	b.write_w(20, 0x10_00_40_61); // 61 40 00 10: (RM)   LA    4,  0: 0,    #16
	b.write_h(24, 0x41_1D);       // 1D 41      : (RR)   SRQ   4,           #1
	b.write_h(26, 0x50_22);       // 22 50      : (RR)   LF    5,     0
	
	b.write_w(28, 0xFF_0F_60_61); // 61 60 0F FF: (RM)   LA    6,  0: 0,    #-1
	b.write_h(32, 0x61_1C);       // 1C 61      : (RR)   SLQ   6,           #1
	b.write_h(34, 0x70_22);       // 22 70      : (RR)   LF    7,     0
	
	b.write_w(36, 0xFF_0F_80_61); // 61 80 0F FF: (RM)   LA    8,  0: 0,    #-1
	b.write_h(40, 0x81_1D);       // 1D 81      : (RR)   SRQ   8,           #1
	b.write_h(42, 0x90_22);       // 22 90      : (RR)   LF    9,     0
	
	b.write_w(44, 0xFC_0F_A0_61); // 61 A0 0F FC: (RM)   LA   10,  0: 0,    #-4
	b.write_h(48, 0xA1_1F);       // 1F A1      : (RR)   ASRQ 10,           #1
	b.write_h(50, 0xB0_22);       // 22 B0      : (RR)   LF   11,     0
	
	b.write_h(52, 0xC1_01);       // 01 C1      : (RR)   LQ   12,           #1
	b.write_h(54, 0x8C_23);       // 23 8C      : (RR)   SF    8,    12
	b.write_h(56, 0x3C_27);       // 27 3C      : (RR)   SSEL  3,    12
	b.write_h(58, 0x8C_23);       // 23 8C      : (RR)   SF    8,    12
	// the CPU should reach an Application Fault at this point
	
	b.write_w(0xC00, 0xFFFFFFFF);
	
	b.write_w(0xF0C, 0xDEADBEEF);
	b.write_w(0xF10, 0x1C07FEFE);
	b.write_b(0xF14, 0xAB);
	b.write_b(0xF15, 0xCD);
	
	let bus = Arc::new(Mutex::new(b));
	let bus2 = Arc::clone(&bus);
	
	let mut running = Arc::clone(&cpu.running);
		
	let arc = Arc::new(Mutex::new(cpu));
	
	SeriesQ::run(Arc::clone(&arc), Arc::clone(&bus2));
	thread::sleep(time::Duration::from_millis(1000));
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
