use std::sync::atomic::Ordering;

use backend_contracts::auth::token_fingerprint;
use redact::Secret;
use sc_mobile::{Caller, MobileReply, MobileRequest};
use tracing::warn;

use super::{CAPTCHA, ERROR, Link, Mobile, REFUSED, guard};

const UNAUTHORIZED: u16 = 401;
const FORBIDDEN: u16 = 403;
const NOT_FOUND: u16 = 404;

impl Link {
    pub(super) async fn send(
        &self,
        mobile: &Mobile,
        op: &'static str,
        request: &MobileRequest,
    ) -> Result<MobileReply, &'static str> {
        if !self.fresh.load(Ordering::Relaxed) {
            self.renew(mobile, None).await?;
        }
        let reply = self.once(op, request).await?;
        if reply.status != UNAUTHORIZED {
            return judged(reply);
        }
        let rejected = token_fingerprint(self.token().expose());
        self.renew(mobile, Some(rejected)).await?;
        judged(self.once(op, request).await?)
    }

    async fn once(
        &self,
        op: &'static str,
        request: &MobileRequest,
    ) -> Result<MobileReply, &'static str> {
        let caller = Caller::user(&self.session.device_id, self.token());
        let answer = self.client.call(&caller, request).await;
        let class = match &answer {
            Ok(reply) => reply.status_class(),
            Err(error) => error.status_class(),
        };
        crate::metrics::record_mobile_request(op, class);
        let reply = answer.map_err(|error| {
            warn!("[mobile] {op} failed: {error}");
            ERROR
        })?;
        if let Some(plan) = &reply.plan {
            self.note_plan(plan);
        }
        Ok(reply)
    }

    async fn renew(&self, mobile: &Mobile, rejected: Option<String>) -> Result<(), &'static str> {
        let old = self.token();
        match (mobile.refresh)(self.session.connection_id, rejected).await {
            Some(token) if token != old => {
                *guard(&self.token) = token;
                self.fresh.store(true, Ordering::Relaxed);
                Ok(())
            }
            _ => Err(REFUSED),
        }
    }

    fn token(&self) -> Secret<String> {
        guard(&self.token).clone()
    }
}

fn judged(reply: MobileReply) -> Result<MobileReply, &'static str> {
    if reply.is_captcha() {
        return Err(CAPTCHA);
    }
    if matches!(reply.status, UNAUTHORIZED | FORBIDDEN | NOT_FOUND) {
        return Err(REFUSED);
    }
    Ok(reply)
}
