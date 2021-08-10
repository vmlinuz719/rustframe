use std::sync::{Arc, Mutex, Condvar};
use std::{thread, time};
mod bus;

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

pub struct Channel {
	brq: Arc<(Mutex<bool>, Condvar)>,
	bgr: Arc<(Mutex<bool>, Condvar)>
}

pub fn new_channel() -> Channel {
	Channel {
		brq: Arc::new((Mutex::new(false), Condvar::new())),
		bgr: Arc::new((Mutex::new(false), Condvar::new()))
	}
}

pub fn clone_channel(ch: Channel) -> Channel {
	Channel {
		brq: Arc::clone(&(ch.brq)),
		bgr: Arc::clone(&(ch.bgr))
	}
}

pub fn call_channel<T>(b: &Arc<Mutex<Bus>>, ch: &Channel, f: fn(&mut Bus) -> T) -> T {
	let &(ref rlock, ref rcvar) = &*(ch.brq);
	let &(ref glock, ref gcvar) = &*(ch.bgr);
	
	// assert BRQ
	let mut rq = rlock.lock().unwrap();
	*rq = true;
	drop(rq);
	
	// wait for BGR
	let mut gr = glock.lock().unwrap();
	while !*gr {
		gr = gcvar.wait(gr).unwrap();
	}
	
	// acquire bus and call f
	let mut bus = b.lock().unwrap();
	let result = f(&mut *bus);
	drop(bus);
	
	let mut rq = rlock.lock().unwrap();
	*rq = false;
	rcvar.notify_one();
	drop(rq);
	
	result
}

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
	let channel_mask = 16;
	
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
	
	thread::spawn(move || {
		let mut bus = cpu_bus.lock().unwrap();
		let mut cycles = 0;
		let mut result;
		
		loop {
			cycles += 1;
			result = collatz(670617279);
			
			for n in 0..N_CHANNELS - channel_mask {
				// test bus request (BRQn) line
				let &(ref rlock, ref rcvar) = &*(cpu_brq[n]);
				let mut rq = rlock.lock().unwrap();
				let request = *rq;
				drop(rq);
				
				if request {
					println!("CPU: Got BRQ on channel {0}, asserting BGR - \
						BEGIN DMA after {1} CPU cycles ({2})", n, cycles, result);
					cycles = 0;
					
					// drop bus
					drop(bus);
					println!("CPU: Bus relinquished");
					
					// assert BGR
					let &(ref glock, ref gcvar) = &*(cpu_bgr[n]);
					let mut gr = glock.lock().unwrap();
					*gr = true;
					gcvar.notify_one();
					drop(gr);
					
					// wait for BRQ to fall
					rq = rlock.lock().unwrap();
					while *rq {
						rq = rcvar.wait(rq).unwrap();
					}
					
					println!("CPU: Lost BRQ on channel {0}, releasing BGR", n);
					// release BGR
					gr = glock.lock().unwrap();
					*gr = false;
					drop(gr);
					
					// reacquire bus
					bus = cpu_bus.lock().unwrap();
					println!("CPU: Bus recovered - END DMA \n");
				}
			}
		}
    });
	
	let ch0_bus = Arc::clone(&bus);
	let ch0_channel = Channel {
		brq: Arc::clone(&(brq[0])),
		bgr: Arc::clone(&(bgr[0]))
	};
	
	loop {
		thread::sleep(time::Duration::from_millis(1000));
		call_channel(&ch0_bus, &ch0_channel, |bus| {
			bus.set_address(0x1C07FEFE);
			bus.read()
		});
	}
}