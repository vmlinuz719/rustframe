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
	
	let mem = Arc::new(Mutex::new(vec![0 as u8; 65536]));
	let mem_clone = Arc::clone(&mem);
	let mut b = bus::Bus::new();
	b.attach(0, 65535, mem_clone);
	
	let bus = Arc::new(Mutex::new(b));
	let bus2 = Arc::clone(&bus);
	
	let mut cpu = cpu::SeriesQ::new(bus);
	let channel = bus::Channel::clone(&cpu.channels[0]);
	
	cpu.R[15] = 0x00001000;
	
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
	
	// test LBA entry
	bus3.write_w(0xC070, 0xDEADBEEF);
	bus3.write_w(0xC074, 0xDEADBEEF);
	bus3.write_b(0xC078, 0xFA);
	bus3.write_b(0xC079, 0xE0);
	bus3.write_b(0xC07A, 0x70);
	bus3.write_b(0xC07B, 0xFF);
	bus3.write_w(0xC07C, 0x1C07FEFE);
	
	// test EBA entry
	bus3.write_w(0xC170, 0x00002000);
	bus3.write_w(0xC174, 0x00002FFF);
	bus3.write_b(0xC178, 0xFF);
	bus3.write_b(0xC179, 0x00);
	bus3.write_b(0xC17A, 0x7E);
	bus3.write_b(0xC17B, 0x00);
	bus3.write_w(0xC17C, 0x00000000);
	
	bus3.write_w(0xF24, 0x00000000);
	bus3.write_w(0xF28, 0x00010000);
	bus3.write_b(0xF2C, 0xEB);
	bus3.write_b(0xF30, 0x01);
	
	bus3.write_h(0x1000, 0x1F_01);	// LFI 1, 15
	bus3.write_h(0x1002, 0x17_1C);  // SLFI 1, 7
	bus3.write_h(0x1004, 0x24_01);	// LFI 2, 4
	bus3.write_h(0x1006, 0x12_25);	// SSBA 1, 2
	bus3.write_h(0x1008, 0x72_2B);	// SLSFI 7, 2
	bus3.write_h(0x100A, 0x03_2B);	// SLSFI 0, 3
	bus3.write_h(0x100A, 0x00_30);	// PLR
	
	bus3.write_h(0x2000, 0x31_0C);	// AFI 3, 1
	bus3.write_h(0x2002, 0x00_30);	// PLR
	
	drop(bus3);
	
	let mut running = Arc::clone(&cpu.running);
		
	let arc = Arc::new(Mutex::new(cpu));
	
	SeriesQ::run(Arc::clone(&arc));
	thread::sleep(time::Duration::from_millis(500));
	
	// let mut x = 0;
	// channel.in_channel(|bus: &mut bus::Bus| -> () {
		// x = bus.read_w(0x0000F000).unwrap();
	// });
	// println!("DMA: Got 0x{:08X}", x);
	
	thread::sleep(time::Duration::from_millis(500));
	running.store(false, Ordering::Relaxed);
	thread::sleep(time::Duration::from_millis(1000));
	
	let c = arc.lock().unwrap();
	println!("R1  : 0x{:08X}", c.R[1]);
	println!("R2  : 0x{:08X}", c.R[2]);
	println!("R3  : 0x{:08X}", c.R[3]);
	println!("R4  : 0x{:08X}", c.R[4]);
	println!("R5  : 0x{:08X}", c.R[5]);
	println!("R6  : 0x{:08X}", c.R[6]);
	println!("R7  : 0x{:08X}", c.R[7]);
	println!("R8  : 0x{:08X}", c.R[8]);
	println!("R9  : 0x{:08X}", c.R[9]);
	println!("R10 : 0x{:08X}", c.R[10]);
	println!("R11 : 0x{:08X}", c.R[11]);
	println!("R12 : 0x{:08X}", c.R[12]);
	println!("R13 : 0x{:08X}", c.R[13]);
	println!("LR  : 0x{:08X}", c.R[14]);
	println!("PC  : 0x{:08X}", c.R[15]);
	println!("SR8 : 0b{:08b}", c.F[8]);
	println!("SSR7: 0x{:02X} (0x{:08X}->0x{:08X}; 0x{:02X}, 0x{:02X})", c.S_selector[7], c.S_base[7], c.S_limit[7], c.S_key[7], c.S_flags[7]);
}
