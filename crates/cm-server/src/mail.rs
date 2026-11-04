//! Delivers the email outbox. With `SMTP_URL` set (e.g. `smtp://localhost:1025`
//! for Mailpit, `smtps://user:pass@host` in production) mail is sent over SMTP;
//! without it, messages are logged and marked sent so development needs no mail server.

use cm_store::sqlx::PgPool;
use lettre::message::Mailbox;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use std::time::Duration;

pub fn spawn_worker(pool: PgPool, smtp_url: Option<String>, from: String) {
    tokio::spawn(async move {
        let transport = match smtp_url
            .as_deref()
            .map(AsyncSmtpTransport::<Tokio1Executor>::from_url)
        {
            Some(Ok(b)) => Some(b.build()),
            Some(Err(e)) => {
                tracing::error!(error = %e, "invalid SMTP_URL; emails will only be logged");
                None
            }
            None => None,
        };
        let from: Option<Mailbox> = from.parse().ok();
        loop {
            match cm_store::accounts::claim_emails(&pool, 20).await {
                Ok(batch) => {
                    for m in batch {
                        let result = deliver(transport.as_ref(), from.as_ref(), &m).await;
                        if let Err(e) = &result {
                            tracing::warn!(to = %m.to_addr, error = %e, "email delivery failed");
                        }
                        let _ = cm_store::accounts::mark_email_result(
                            &pool,
                            m.id,
                            result.err().as_deref(),
                        )
                        .await;
                    }
                }
                Err(e) => tracing::error!(error = %e, "outbox claim failed"),
            }
            tokio::time::sleep(Duration::from_secs(3)).await;
        }
    });
}

async fn deliver(
    transport: Option<&AsyncSmtpTransport<Tokio1Executor>>,
    from: Option<&Mailbox>,
    m: &cm_store::accounts::OutboxEmail,
) -> Result<(), String> {
    let Some(transport) = transport else {
        tracing::info!(to = %m.to_addr, subject = %m.subject, body = %m.body_text, "email (SMTP not configured)");
        return Ok(());
    };
    let from = from.ok_or("MAIL_FROM is not a valid address")?.clone();
    let to: Mailbox = m
        .to_addr
        .parse()
        .map_err(|e| format!("bad recipient: {e}"))?;
    let msg = Message::builder()
        .from(from)
        .to(to)
        .subject(&m.subject)
        .body(m.body_text.clone())
        .map_err(|e| e.to_string())?;
    transport
        .send(msg)
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}
