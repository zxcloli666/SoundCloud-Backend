mod review;

use super::director::{Brief, Director, Plan, Replan, Reshot, Review, Verdict};
use crate::llm::anthropic::{Answer, Anthropic};

impl Director for Anthropic {
    async fn plan(&self, brief: &Brief, refused: Option<&str>) -> Answer<Plan> {
        let mut spend = Vec::new();
        let result = review::plan(self, brief, refused, &mut spend).await;
        Answer { result, spend }
    }

    async fn replan(&self, replan: &Replan<'_>) -> Answer<Reshot> {
        let mut spend = Vec::new();
        let result = review::replan(self, replan, &mut spend).await;
        Answer { result, spend }
    }

    async fn review(&self, review: &Review<'_>) -> Answer<Verdict> {
        let mut spend = Vec::new();
        let result = review::verdict(self, review, &mut spend).await;
        Answer { result, spend }
    }
}
