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
	
	b.write_w(0,  0x01_00_20_61); // 61 20 00 01: (RM)   LA    2, 0: 0,    I#1
	b.write_w(4,  0x10_00_10_61); // 61 10 00 10: (RM)   LA    1, 0: 0,    I#16
	b.write_h(8,  0x12_0A);       // 0A 12      : (RR)   S     1,    2
	b.write_h(10, 0x02_3E);       // 3E 12      : (SK)   IFS
	b.write_w(12, 0xF8_FF_FF_61); // 61 FF FF FA: (RM)   LA   PC,PS:PC,    I#-8
	b.write_w(16, 0xFF_FF_FF_FF); // FF FF FF FF:        ILLEGAL
	
	b.write_w(0xFFC, 0xF00FD00D);
	
	let bus = Arc::new(Mutex::new(b));
	let bus2 = Arc::clone(&bus);
	
	let mut running = Arc::clone(&cpu.running);
		
	let arc = Arc::new(Mutex::new(cpu));
	
	SeriesQ::run(Arc::clone(&arc), Arc::clone(&bus2));
	thread::sleep(time::Duration::from_millis(1000));
	running.store(false, Ordering::Relaxed);
	thread::sleep(time::Duration::from_millis(1000));
	
	let c = arc.lock().unwrap();
	println!("R1: 0x{:08X}", c.R[1]);
	println!("R2: 0x{:08X}", c.R[2]);
	println!("R3: 0x{:08X}", c.R[3]);
	println!("R4: 0x{:08X}", c.R[4]);
	println!("R5: 0x{:08X}", c.R[5]);
	println!("R6: 0x{:08X}", c.R[6]);
	println!("R7: 0x{:08X}", c.R[7]);
	println!("R8: 0x{:08X}", c.R[8]);
}
