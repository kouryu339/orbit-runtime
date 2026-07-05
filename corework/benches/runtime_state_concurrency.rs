use corework::cache::InMemoryCache;
use corework::ecs::{EcsWorld, SpawnUnitCommand};
use corework::event_line::EventLinePolicy;
use corework::execution_unit::UnitType;
use corework::runtime_state::{MapRuntimeStateStore, RuntimeStateStore};
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::hint::black_box;
use std::sync::{Arc, Barrier};

const OPERATIONS_PER_THREAD: usize = 4_096;
const THREAD_COUNTS: [usize; 4] = [1, 4, 16, 32];

fn spawn_unit(store: &MapRuntimeStateStore, unit_id: String) {
    store.units().spawn_unit(SpawnUnitCommand {
        cache_scope_id: format!("scope:bench:{unit_id}"),
        unit_id,
        unit_type: UnitType::Module,
        parent_id: None,
        ancestor_ids: Vec::new(),
        scope_id: "scope:bench".to_string(),
        conversation_id: None,
    });
}

fn build_store(thread_count: usize, shared_owner: bool) -> Arc<MapRuntimeStateStore> {
    let store = Arc::new(MapRuntimeStateStore::new(
        Arc::new(InMemoryCache::new()),
        Arc::new(EcsWorld::new()),
    ));
    if shared_owner {
        spawn_unit(&store, "unit:shared".to_string());
    } else {
        for thread_index in 0..thread_count {
            spawn_unit(&store, format!("unit:{thread_index}"));
        }
    }
    store
}

fn run_batch(store: &Arc<MapRuntimeStateStore>, thread_count: usize, shared_owner: bool) {
    let barrier = Arc::new(Barrier::new(thread_count));
    std::thread::scope(|scope| {
        for thread_index in 0..thread_count {
            let store = Arc::clone(store);
            let barrier = Arc::clone(&barrier);
            scope.spawn(move || {
                let owner = if shared_owner {
                    "unit:shared".to_string()
                } else {
                    format!("unit:{thread_index}")
                };
                let line = format!("line:{thread_index}");
                barrier.wait();
                for _ in 0..OPERATIONS_PER_THREAD {
                    black_box(store.events().declare_line(
                        &owner,
                        &line,
                        EventLinePolicy::private(),
                    ));
                    black_box(
                        store
                            .shared_components()
                            .provide(&owner, std::any::type_name::<String>()),
                    );
                }
            });
        }
    });
}

fn benchmark_runtime_state_concurrency(c: &mut Criterion) {
    let mut group = c.benchmark_group("runtime_state_mixed_writes");
    for thread_count in THREAD_COUNTS {
        let operation_count = (thread_count * OPERATIONS_PER_THREAD) as u64;
        group.throughput(Throughput::Elements(operation_count));

        let sharded_store = build_store(thread_count, false);
        group.bench_with_input(
            BenchmarkId::new("sharded_owners", thread_count),
            &thread_count,
            |b, &thread_count| {
                b.iter(|| run_batch(&sharded_store, thread_count, false));
            },
        );

        let hot_store = build_store(thread_count, true);
        group.bench_with_input(
            BenchmarkId::new("single_hot_owner", thread_count),
            &thread_count,
            |b, &thread_count| {
                b.iter(|| run_batch(&hot_store, thread_count, true));
            },
        );
    }
    group.finish();
}

criterion_group!(benches, benchmark_runtime_state_concurrency);
criterion_main!(benches);
