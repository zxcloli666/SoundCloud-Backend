use futures::future::join_all;
use sqlx::{Postgres, Transaction};

use super::catalog::{SearchService, configured};
use super::resolve::Resolved;
use crate::error::AppResult;

pub const IDLE_KEPT: usize = 2;

pub struct Lane {
    pub tx: Transaction<'static, Postgres>,
    pub found: Vec<Resolved>,
}

impl SearchService {
    pub(super) async fn lanes(&self, wanted: usize) -> AppResult<Vec<Lane>> {
        let own = self.begin().await?;
        let spare = join_all((1..wanted).map(|_| self.spare())).await;
        let open = std::iter::once(own).chain(spare.into_iter().flatten());
        Ok(open
            .map(|tx| Lane {
                tx,
                found: Vec::new(),
            })
            .collect())
    }

    async fn spare(&self) -> Option<Transaction<'static, Postgres>> {
        if self.pg.num_idle() <= IDLE_KEPT {
            return None;
        }
        let tx = self.pg.try_begin().await.ok()??;
        configured(tx).await.ok()
    }
}
