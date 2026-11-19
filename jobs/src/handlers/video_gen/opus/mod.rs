mod review;

use super::director::{Brief, Director, Plan, Review, Verdict};
use crate::llm::anthropic::{Answer, Anthropic};

impl Director for Anthropic {
    async fn plan(&self, brief: &Brief) -> Answer<Plan> {
        let mut spend = Vec::new();
        let result = review::plan(self, brief, &mut spend).await;
        Answer { result, spend }
    }

    async fn review(&self, review: &Review<'_>) -> Answer<Verdict> {
        let mut spend = Vec::new();
        let result = review::verdict(self, review, &mut spend).await;
        Answer { result, spend }
    }
}
