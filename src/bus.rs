use std::sync::{Arc, Mutex, Condvar};

pub struct Bus {
	address: u32,
	data: u32
}

impl Bus {
	pub fn new() -> Bus {
		Bus {
			address: 0,
			data: 0
		}
	}
	
	pub fn set_address(&mut self, address: u32) {
		self.address = address;
	}
	
	pub fn read(&self) -> u32 {
		println!("Bus: Read 0x{:08X}", self.address);
		return self.data;
	}
}

pub struct Channel {
	bus: Arc<Mutex<Bus>>,
	brq: Arc<(Mutex<bool>, Condvar)>,
	bgr: Arc<(Mutex<bool>, Condvar)>
}

impl Channel {
	pub fn new(bus: &Arc<Mutex<Bus>>) -> Channel {
		Channel {
			bus: Arc::clone(&bus),
			brq: Arc::new((Mutex::new(false), Condvar::new())),
			bgr: Arc::new((Mutex::new(false), Condvar::new()))
		}
	}
	
	pub fn clone(ch: &Channel) -> Channel {
		Channel {
			bus: Arc::clone(&ch.bus),
			brq: Arc::clone(&ch.brq),
			bgr: Arc::clone(&ch.bgr)
		}
	}
	
	pub fn in_channel<T>(&self, f: fn(&mut Bus) -> T) -> T {
		let &(ref rlock, ref rcvar) = &*(self.brq);
		let &(ref glock, ref gcvar) = &*(self.bgr);
		
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
		let mut bus = self.bus.lock().unwrap();
		let result = f(&mut *bus);
		drop(bus);
		
		// release BRQ
		let mut rq = rlock.lock().unwrap();
		*rq = false;
		rcvar.notify_one();
		drop(rq);
		
		result
	}
	
	pub fn check_pending(&self) -> bool {
		// test bus request (BRQn) line
		let &(ref rlock, _) = &*(self.brq);
		let rq = rlock.lock().unwrap();
		let result = *rq;
		drop(rq);
		
		result
	}
	
	pub fn open(&self) {
		// Note: Caller must relinquish bus and reacquire after calling open
		
		let &(ref rlock, ref rcvar) = &*(self.brq);
		let &(ref glock, ref gcvar) = &*(self.bgr);
		
		// assert BGR
		let mut gr = glock.lock().unwrap();
		*gr = true;
		gcvar.notify_one();
		drop(gr);
		
		// wait for BRQ to fall
		let mut rq = rlock.lock().unwrap();
		while *rq {
			rq = rcvar.wait(rq).unwrap();
		}
		
		// release BGR
		gr = glock.lock().unwrap();
		*gr = false;
		drop(gr);
	}
}