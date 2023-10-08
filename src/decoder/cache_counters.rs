use once_cell::sync::OnceCell;
use perfcnt::linux::{CacheId, CacheOpId, CacheOpResultId, PerfCounter, PerfCounterBuilderLinux};
use perfcnt::AbstractPerfCounter;
use std::sync::{Mutex, MutexGuard};

struct CacheCounters {
    l1_read_misses: PerfCounter,
    //l1_write_misses: PerfCounter,
}

impl CacheCounters {
    fn new() -> Self {
        let l1_read_misses = PerfCounterBuilderLinux::from_cache_event(
            CacheId::L1D,
            CacheOpId::Read,
            CacheOpResultId::Miss,
        )
        .finish()
        .expect("Could not create the counter");
        /*
        let l1_write_misses = PerfCounterBuilderLinux::from_cache_event(
                CacheId::L1D,
                CacheOpId::Write,
                CacheOpResultId::Miss,
            ).finish().expect("Could not create the counter");
            */
        Self { l1_read_misses } //, l1_write_misses }
    }

    fn start(&mut self) {
        self.l1_read_misses.reset().unwrap();
        //self.l1_write_misses.reset().unwrap();
    }

    fn stop(&mut self) -> u64 {
        self.l1_read_misses.read().unwrap() // + self.l1_write_misses.read().unwrap()
    }
}

fn get_counters() -> MutexGuard<'static, CacheCounters> {
    static INSTANCE: OnceCell<Mutex<CacheCounters>> = OnceCell::new();
    INSTANCE
        .get_or_init(|| Mutex::new(CacheCounters::new()))
        .lock()
        .unwrap()
}

pub fn start() {
    get_counters().start()
}

pub fn stop() -> u64 {
    get_counters().stop()
}
