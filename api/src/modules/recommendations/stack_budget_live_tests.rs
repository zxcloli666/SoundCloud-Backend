use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;

use super::home_wave::HomeRequest;
use super::live_fixture::{
    LISTENER, PER_CLUSTER, VECTOR_CLUSTERS, catalogue, install_all_vectors, install_catalog,
    likes_the_same_ten, service,
};

const TOKIO_WORKER_STACK: usize = 2 * 1024 * 1024;
const HANDLER_STACK_BUDGET: usize = TOKIO_WORKER_STACK / 2;
const HANDLER_FUTURE_BUDGET: usize = 4 * 1024;

struct BoundedRun {
    future_size: usize,
    page: String,
}

fn request() -> HomeRequest {
    HomeRequest {
        sc_user_id: LISTENER.to_owned(),
        languages: None,
        per_cluster: PER_CLUSTER,
        hide_listened: false,
    }
}

fn home_page_on_a_bounded_stack(pg: &PgPool, stack: usize) -> anyhow::Result<BoundedRun> {
    let options = pg.connect_options().as_ref().clone();
    let worker = std::thread::Builder::new()
        .name("bounded-stack-worker".to_owned())
        .stack_size(stack)
        .spawn(move || -> anyhow::Result<BoundedRun> {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            runtime.block_on(async move {
                let pg = PgPoolOptions::new()
                    .max_connections(8)
                    .connect_with(options)
                    .await?;
                let service = service(pg).await?;
                let handled = tokio::spawn(async move {
                    let page = service.home_wave_coalesced(request());
                    let future_size = std::mem::size_of_val(&page);
                    page.await.map(|page| BoundedRun { future_size, page })
                });
                Ok(handled.await??)
            })
        })?;
    worker
        .join()
        .map_err(|_| anyhow::anyhow!("the home page worker panicked"))?
}

fn shelves(page: &str) -> anyhow::Result<Vec<String>> {
    let page: serde_json::Value = serde_json::from_str(page)?;
    Ok(page["clusters"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|cluster| cluster["id"].as_str().map(str::to_owned))
        .collect())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Qdrant, Redis and NATS"]
async fn the_home_page_of_a_listener_with_history_fits_in_half_a_tokio_worker_stack(
    pg: PgPool,
) -> anyhow::Result<()> {
    let tracks = catalogue(960_000, 60);
    install_catalog(&pg, &tracks).await?;
    likes_the_same_ten(&pg, LISTENER, 960_000).await?;
    let seeding = service(pg.clone()).await?;
    install_all_vectors(&seeding, &tracks).await?;

    let worker_pg = pg.clone();
    let run = tokio::task::spawn_blocking(move || {
        home_page_on_a_bounded_stack(&worker_pg, HANDLER_STACK_BUDGET)
    })
    .await??;

    let built = shelves(&run.page)?;
    for expected in VECTOR_CLUSTERS {
        assert!(
            built.iter().any(|shelf| shelf == expected),
            "`{expected}` is missing, so the deepest path of the home page never ran and the \
             stack it needs was not measured: {built:?}"
        );
    }
    assert!(
        run.future_size <= HANDLER_FUTURE_BUDGET,
        "the handler future is {} bytes: every caller between hyper and the page inlines it, \
         and an unoptimized build gives each of their poll frames a copy of that size",
        run.future_size
    );
    Ok(())
}
