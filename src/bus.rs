use std::sync::{Arc, Mutex, Condvar};

// Memory32 trait for use with bus, as well as reference impl for Vec<u8>

#[derive(Debug)]
#[allow(dead_code)]
pub enum BusError {
	AccessViolation,
	AlignmentCheck,
	InvalidAddress
}

pub trait Memory32<A, E> {
	fn read_b(&self, addr: A) -> Result<u8, E>;
	fn read_h(&self, addr: A) -> Result<u16, E>;
	fn read_w(&self, addr: A) -> Result<u32, E>;
	
	fn write_b(&mut self, addr: A, data: u8) -> Result<(), E>;
	fn write_h(&mut self, addr: A, data: u16) -> Result<(), E>;
	fn write_w(&mut self, addr: A, data: u32) -> Result<(), E>;
}

impl Memory32<u32, BusError> for Vec<u8> {
	fn read_b(&self, addr: u32) -> Result<u8, BusError> {
		if addr >= self.len() as u32 {
			Err(BusError::InvalidAddress)
		} else {
			Ok(self[addr as usize])
		}
	}
	fn read_h(&self, addr: u32) -> Result<u16, BusError> {
		if addr + 1 >= self.len() as u32 {
			Err(BusError::InvalidAddress)
		} else if addr % 2 != 0 {
			Err(BusError::AlignmentCheck)
		} else {
			Ok(((self[(addr + 1) as usize] as u16) << 8) + (self[addr as usize] as u16))
		}
	}
	fn read_w(&self, addr: u32) -> Result<u32, BusError> {
		if addr + 3 >= self.len() as u32 {
			Err(BusError::InvalidAddress)
		} else if addr % 4 != 0 {
			Err(BusError::AlignmentCheck)
		} else {
			Ok(((self[(addr + 3) as usize] as u32) << 3) + ((self[(addr + 2) as usize] as u32) << 8)
				+ ((self[(addr + 1) as usize] as u32) << 8) + (self[addr as usize] as u32))
		}
	}
	
	fn write_b(&mut self, addr: u32, data: u8) -> Result<(), BusError> {
		if addr >= self.len() as u32 {
			Err(BusError::InvalidAddress)
		} else {
			self[addr as usize] = data;
			Ok(())
		}
	}
	fn write_h(&mut self, addr: u32, data: u16) -> Result<(), BusError> {
		if addr + 1 >= self.len() as u32 {
			Err(BusError::InvalidAddress)
		} else if addr % 2 != 0 {
			Err(BusError::AlignmentCheck)
		} else {
			self[(addr + 1) as usize] = ((data >> 8) & 0xFF) as u8;
			self[addr as usize] = (data & 0xFF) as u8;
			Ok(())
		}
	}
	fn write_w(&mut self, addr: u32, data: u32) -> Result<(), BusError> {
		if addr + 3 >= self.len() as u32 {
			Err(BusError::InvalidAddress)
		} else if addr % 4 != 0 {
			Err(BusError::AlignmentCheck)
		} else {
			self[(addr + 3) as usize] = ((data >> 24) & 0xFF) as u8;
			self[(addr + 2) as usize] = ((data >> 16) & 0xFF) as u8;
			self[(addr + 1) as usize] = ((data >> 8) & 0xFF) as u8;
			self[addr as usize] = (data & 0xFF) as u8;
			Ok(())
		}
	}
}

// Bus: Attach and access multiple Memory32 simulated devices

pub struct Bus {
	base: Vec<u32>,
	size: Vec<u32>,
	region: Vec<Arc<Mutex<dyn Memory32<u32, BusError> + Send>>>
}

impl Bus {
	pub fn new() -> Bus {
		Bus {
			base: Vec::new(),
			size: Vec::new(),
			region: Vec::new()
		}
	}
	
	pub fn attach(&mut self, base: u32, size: u32,
		region: Arc<Mutex<dyn Memory32<u32, BusError> + Send>>) {
		self.base.push(base);
		self.size.push(size);
		self.region.push(region);
	}
}

impl Memory32<u32, BusError> for Bus {
	fn read_b(&self, addr: u32) -> Result<u8, BusError> {
		for n in 0..self.base.len() {
			if addr >= self.base[n] && addr < self.base[n] + self.size[n] {
				let mem = self.region[n].lock().unwrap();
				return mem.read_b(addr - self.base[n]);
			}
		}
		return Err(BusError::InvalidAddress);
	}
	fn read_h(&self, addr: u32) -> Result<u16, BusError> {
		for n in 0..self.base.len() {
			if addr >= self.base[n] && addr < self.base[n] + self.size[n] {
				let mem = self.region[n].lock().unwrap();
				return mem.read_h(addr - self.base[n]);
			}
		}
		return Err(BusError::InvalidAddress);
	}
	fn read_w(&self, addr: u32) -> Result<u32, BusError> {
		for n in 0..self.base.len() {
			if addr >= self.base[n] && addr < self.base[n] + self.size[n] {
				let mem = self.region[n].lock().unwrap();
				return mem.read_w(addr - self.base[n]);
			}
		}
		return Err(BusError::InvalidAddress);
	}
	
	fn write_b(&mut self, addr: u32, data: u8) -> Result<(), BusError> {
		for n in 0..self.base.len() {
			if addr >= self.base[n] && addr < self.base[n] + self.size[n] {
				let mut mem = self.region[n].lock().unwrap();
				return mem.write_b(addr - self.base[n], data);
			}
		}
		return Err(BusError::InvalidAddress);
	}
	fn write_h(&mut self, addr: u32, data: u16) -> Result<(), BusError> {
		for n in 0..self.base.len() {
			if addr >= self.base[n] && addr < self.base[n] + self.size[n] {
				let mut mem = self.region[n].lock().unwrap();
				return mem.write_h(addr - self.base[n], data);
			}
		}
		return Err(BusError::InvalidAddress);
	}
	fn write_w(&mut self, addr: u32, data: u32) -> Result<(), BusError> {
		for n in 0..self.base.len() {
			if addr >= self.base[n] && addr < self.base[n] + self.size[n] {
				let mut mem = self.region[n].lock().unwrap();
				return mem.write_w(addr - self.base[n], data);
			}
		}
		return Err(BusError::InvalidAddress);
	}
}

// Channel - a generic synchronization construct

pub struct Channel<T> {
	bus: Arc<Mutex<T>>,
	brq: Arc<(Mutex<bool>, Condvar)>,
	bgr: Arc<(Mutex<bool>, Condvar)>
}

impl<T> Channel<T> {
	pub fn new(bus: &Arc<Mutex<T>>) -> Channel<T> {
		Channel {
			bus: Arc::clone(&bus),
			brq: Arc::new((Mutex::new(false), Condvar::new())),
			bgr: Arc::new((Mutex::new(false), Condvar::new()))
		}
	}
	
	pub fn clone(ch: &Channel<T>) -> Channel<T> {
		Channel {
			bus: Arc::clone(&ch.bus),
			brq: Arc::clone(&ch.brq),
			bgr: Arc::clone(&ch.bgr)
		}
	}
	
	pub fn in_channel<U>(&self, f: fn(&mut T) -> U) -> U {
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
