use corework::cache::InMemoryCache;
use corework::ecs::{EcsWorld, SpawnUnitCommand};
use corework::event_line::EventLinePolicy;
use corework::execution_unit::UnitType;
use corework::runtime_state::{MapRuntimeStateStore, RuntimeStateStore};
use std::hint::black_box;
use std::sync::{Arc, Barrier};
use std::time::{Duration, Instant};

const THREADS: usize = 32;
const UNIQUE_WRITES_PER_THREAD: usize = 10_000;
const MIXED_OPERATIONS_PER_THREAD: usize = 100_000;
const PREPOPULATED_LINES: usize = 16;
const SAMPLE_EVERY: usize = 256;

#[derive(Debug)]
struct LoadResult {
    operations: usize,
    elapsed: Duration,
    latency_ns: Vec<u64>,
}

impl LoadResult {
    fn print(&mut self, workload: &str, topology: &str) {
        self.latency_ns.sort_unstable();
        let percentile = |p: f64| {
            let index = ((self.latency_ns.len() - 1) as f64 * p).round() as usize;
            self.latency_ns[index]
        };
        println!(
            "{workload:14} {topology:14} {:>10.0} ops/s  p50 {:>8} ns  p95 {:>8} ns  p99 {:>8} ns",
            self.operations as f64 / self.elapsed.as_secs_f64(),
            percentile(0.50),
            percentile(0.95),
            percentile(0.99),
        );
    }
}

fn spawn_unit(store: &MapRuntimeStateStore, unit_id: String) {
    store.units().spawn_unit(SpawnUnitCommand {
        cache_scope_id: format!("scope:load:{unit_id}"),
        unit_id,
        unit_type: UnitType::Module,
        parent_id: None,
        ancestor_ids: Vec::new(),
        scope_id: "scope:load".to_string(),
        conversation_id: None,
    });
}

fn build_store(shared_owner: bool) -> Arc<MapRuntimeStateStore> {
    let store = Arc::new(MapRuntimeStateStore::new(
        Arc::new(InMemoryCache::new()),
        Arc::new(EcsWorld::new()),
    ));
    if shared_owner {
        spawn_unit(&store, "unit:shared".to_string());
    } else {
        for thread_index in 0..THREADS {
            spawn_unit(&store, format!("unit:{thread_index}"));
        }
    }
    store
}

fn owner_id(thread_index: usize, shared_owner: bool) -> String {
    if shared_owner {
        "unit:shared".to_string()
    } else {
        format!("unit:{thread_index}")
    }
}

fn should_sample(thread_index: usize, operation_index: usize) -> bool {
    let mut sample_key = (operation_index as u64)
        .wrapping_add((thread_index as u64) << 32)
        .wrapping_mul(0x9e37_79b9_7f4a_7c15);
    sample_key ^= sample_key >> 30;
    sample_key = sample_key.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    sample_key ^= sample_key >> 27;
    (sample_key as usize).is_multiple_of(SAMPLE_EVERY)
}

fn run_unique_writes(shared_owner: bool) -> LoadResult {
    let store = build_store(shared_owner);
    let barrier = Arc::new(Barrier::new(THREADS + 1));
    let mut latency_ns = Vec::new();
    let elapsed = std::thread::scope(|scope| {
        let handles = (0..THREADS)
            .map(|thread_index| {
                let store = Arc::clone(&store);
                let barrier = Arc::clone(&barrier);
                scope.spawn(move || {
                    let owner = owner_id(thread_index, shared_owner);
                    let mut samples = Vec::new();
                    barrier.wait();
                    for operation_index in 0..UNIQUE_WRITES_PER_THREAD {
                        let sample = should_sample(thread_index, operation_index);
                        let operation_started = sample.then(Instant::now);
                        black_box(store.events().declare_line(
                            &owner,
                            &format!("line:{thread_index}:{operation_index}"),
                            EventLinePolicy::private(),
                        ));
                        if let Some(operation_started) = operation_started {
                            samples.push(operation_started.elapsed().as_nanos() as u64);
                        }
                    }
                    samples
                })
            })
            .collect::<Vec<_>>();
        barrier.wait();
        let started = Instant::now();
        for handle in handles {
            latency_ns.extend(handle.join().unwrap());
        }
        started.elapsed()
    });
    LoadResult {
        operations: THREADS * UNIQUE_WRITES_PER_THREAD,
        elapsed,
        latency_ns,
    }
}

fn run_mixed(shared_owner: bool) -> LoadResult {
    let store = build_store(shared_owner);
    let owners = if shared_owner { 1 } else { THREADS };
    for owner_index in 0..owners {
        let owner = owner_id(owner_index, shared_owner);
        for line_index in 0..PREPOPULATED_LINES {
            assert!(store.events().declare_line(
                &owner,
                &format!("line:{line_index}"),
                EventLinePolicy::private(),
            ));
        }
        assert!(store
            .shared_components()
            .provide(&owner, std::any::type_name::<String>()));
    }

    let barrier = Arc::new(Barrier::new(THREADS + 1));
    let mut latency_ns = Vec::new();
    let elapsed = std::thread::scope(|scope| {
        let handles = (0..THREADS)
            .map(|thread_index| {
                let store = Arc::clone(&store);
                let barrier = Arc::clone(&barrier);
                scope.spawn(move || {
                    let owner = owner_id(thread_index, shared_owner);
                    let mut state = (thread_index as u64 + 1) * 0x9e37_79b9;
                    let mut samples = Vec::new();
                    barrier.wait();
                    for operation_index in 0..MIXED_OPERATIONS_PER_THREAD {
                        state ^= state << 13;
                        state ^= state >> 7;
                        state ^= state << 17;
                        let sample = should_sample(thread_index, operation_index);
                        let operation_started = sample.then(Instant::now);
                        if state.is_multiple_of(10) {
                            black_box(store.events().declare_line(
                                &owner,
                                &format!("line:{}", state as usize % PREPOPULATED_LINES),
                                EventLinePolicy::private(),
                            ));
                            black_box(
                                store
                                    .shared_components()
                                    .provide(&owner, std::any::type_name::<String>()),
                            );
                        } else if state & 1 == 0 {
                            black_box(store.events().lines_of(&owner));
                        } else {
                            black_box(store.shared_components().components_of(&owner));
                        }
                        if let Some(operation_started) = operation_started {
                            samples.push(operation_started.elapsed().as_nanos() as u64);
                        }
                    }
                    samples
                })
            })
            .collect::<Vec<_>>();
        barrier.wait();
        let started = Instant::now();
        for handle in handles {
            latency_ns.extend(handle.join().unwrap());
        }
        started.elapsed()
    });
    LoadResult {
        operations: THREADS * MIXED_OPERATIONS_PER_THREAD,
        elapsed,
        latency_ns,
    }
}

fn main() {
    println!("32 threads; sampled latency every {SAMPLE_EVERY} operations");
    run_unique_writes(false).print("unique writes", "sharded");
    run_unique_writes(true).print("unique writes", "hot owner");
    run_mixed(false).print("90r/10w mixed", "sharded");
    run_mixed(true).print("90r/10w mixed", "hot owner");
}
