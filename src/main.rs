use std::sync::{Arc, Mutex};
use std::{thread, time};
mod bus;

fn collatz(n: u64) -> u64 {
	let mut iterations = 0;
	let mut x = n;
	while x != 1 {
		if x % 2 == 0 {
			x /= 2;
		} else {
			x = x * 3 + 1;
		}
		iterations += 1;
	}
	
	iterations
}

fn main() {
	let bus = Arc::new(Mutex::new(bus::Bus::new()));
	let channels = [bus::Channel::new(&bus), bus::Channel::new(&bus)];
	let ch0 = bus::Channel::clone(&channels[0]);
	let ch1 = bus::Channel::clone(&channels[1]);
	
	// simulate device on channel 0
	thread::spawn(move || {
		loop {
			thread::sleep(time::Duration::from_millis(1000));
			ch0.in_channel(|bus| {
				bus.set_address(0xDEADBEEF);
				bus.read()
			});
			println!("Bus call successful");
		}
	});
	
	// simulate device on channel 1
	thread::spawn(move || {
		loop {
			thread::sleep(time::Duration::from_millis(900));
			ch1.in_channel(|bus| {
				bus.set_address(0x0C07FEFE);
				bus.read()
			});
			println!("Bus call successful");
		}
	});
	
	// simulate bus host
	let mut held_bus = bus.lock().unwrap();
	let mut cycles = 0;
	let mut result;
	
	loop {
		cycles += 1;
		result = collatz(670617279);
		
		for n in 0..2 {
			if channels[n].check_pending() {
				println!("CPU: Got BRQ on channel {0}, asserting BGR - \
					BEGIN DMA after {1} CPU cycles ({2})", n, cycles, result);
				cycles = 0; result = 0;
				
				drop(held_bus);
				channels[n].open();
				held_bus = bus.lock().unwrap();
			}
		}
	}
}