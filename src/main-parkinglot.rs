use parking_lot::{Mutex, Condvar};
use std::sync::Arc;
use std::{thread, time};

const N_CHANNELS: usize = 32;

pub struct Bus {
	address: u32,
	data: u32
}

pub fn new_bus() -> Bus {
	Bus {
		address: 0,
		data: 0
	}
}

impl Bus {
	pub fn set_address(&mut self, address: u32) {
		self.address = address;
	}
	
	pub fn read(&self) -> u32 {
		println!("Bus: Read 0x{:08X}", self.address);
		return self.data;
	}
}

fn main() {
	let bus = Arc::new(Mutex::new(new_bus()));
	let mut brq = Vec::new(); // bus request
	let mut bgr = Vec::new(); // bus grant
	
	for _ in 0..N_CHANNELS {
		brq.push(Arc::new((Mutex::new(false), Condvar::new())));
		bgr.push(Arc::new((Mutex::new(false), Condvar::new())));
	}
	
	let cpu_bus = Arc::clone(&bus);
	let mut cpu_brq = Vec::new();
	let mut cpu_bgr = Vec::new();
		
	for n in 0..N_CHANNELS {
		cpu_brq.push(Arc::clone(&(brq[n])));
		cpu_bgr.push(Arc::clone(&(bgr[n])));
	}
	
	let cpu = thread::spawn(move || {
		let mut bus = cpu_bus.lock();
		let mut cycles = 0;
		
		loop {
			cycles += 1;
			
			for n in 0..N_CHANNELS {
				let &(ref rlock, ref rcvar) = &*(cpu_brq[n]);
				let mut rq = rlock.lock();
				let request = *rq;
				drop(rq);
				
				if request {
					println!("CPU: Got BRQ on channel {0}, asserting BGR - BEGIN DMA after {1} cycles", n, cycles);
					cycles = 0;
					// assert BGR
					let &(ref glock, ref gcvar) = &*(cpu_bgr[n]);
					let mut gr = glock.lock();
					*gr = true;
					gcvar.notify_one();
					drop(gr);
					
					// drop bus
					drop(bus);
					println!("CPU: Bus relinquished");
					
					// wait for BRQ to fall
					rq = rlock.lock();
					while *rq {
						rcvar.wait(&mut rq);
					}
					
					println!("CPU: Lost BRQ on channel {0}, releasing BGR", n);
					// release BGR
					gr = glock.lock();
					*gr = false;
					drop(gr);
					
					// reacquire bus
					bus = cpu_bus.lock();
					println!("CPU: Bus recovered - END DMA \n");
				}
			}
		}
    });
	
	let ch0_bus = Arc::clone(&bus);
	let ch0_brq = Arc::clone(&(brq[0]));
	let ch0_bgr = Arc::clone(&(bgr[0]));
	
	let ch0 = thread::spawn(move || {
		// use channel 0
		let &(ref rlock, ref rcvar) = &*ch0_brq;
		let &(ref glock, ref gcvar) = &*ch0_bgr;
		
        loop {
			// wait 0.5 seconds
			thread::sleep(time::Duration::from_millis(1000));
			
			// assert BRQ
			let mut rq = rlock.lock();
			*rq = true;
			println!("Channel 0: Asserted BRQ");
			drop(rq);
			
			// wait for BGR
			let mut gr = glock.lock();
			while !*gr {
				gcvar.wait(&mut gr);
			}
			
			println!("Channel 0: Got BGR, reading data now");
			// acquire bus
			let mut bus = ch0_bus.lock();
			bus.set_address(0x0C07FEFE);
			bus.read();
			drop(bus);
			
			// release BRQ
			let mut rq = rlock.lock();
			*rq = false;
			rcvar.notify_one();
			drop(rq);
			println!("Channel 0: Released BRQ");
			
		}
    });
	
	let ch1_bus = Arc::clone(&bus);
	let ch1_brq = Arc::clone(&(brq[1]));
	let ch1_bgr = Arc::clone(&(bgr[1]));
	
	let ch1 = thread::spawn(move || {
		// use channel 0
		let &(ref rlock, ref rcvar) = &*ch1_brq;
		let &(ref glock, ref gcvar) = &*ch1_bgr;
		
        loop {
			// wait 0.5 seconds
			thread::sleep(time::Duration::from_millis(1000));
			
			// assert BRQ
			let mut rq = rlock.lock();
			*rq = true;
			println!("Channel 1: Asserted BRQ");
			drop(rq);
			
			// wait for BGR
			let mut gr = glock.lock();
			while !*gr {
				gcvar.wait(&mut gr);
			}
			
			println!("Channel 1: Got BGR, reading data now");
			// acquire bus
			let mut bus = ch1_bus.lock();
			bus.set_address(0x1C07FEFE);
			bus.read();
			drop(bus);
			
			// release BRQ
			let mut rq = rlock.lock();
			*rq = false;
			rcvar.notify_one();
			drop(rq);
			println!("Channel 1: Released BRQ");
			
		}
    });
	
    cpu.join().expect("Impossible error 1: thread execution failure");
	ch0.join().expect("Impossible error 1: thread execution failure");
	ch1.join().expect("Impossible error 1: thread execution failure");
}