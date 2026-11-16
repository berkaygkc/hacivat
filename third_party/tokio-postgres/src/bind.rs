use crate::client::InnerClient;
use crate::codec::FrontendMessage;
use crate::connection::RequestMessages;
use crate::types::BorrowToSql;
use crate::{Error, Portal, Statement, query};
use postgres_protocol::message::backend::Message;
use postgres_protocol::message::frontend;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

pub async fn bind<P, I>(
    client: &Arc<InnerClient>,
    statement: Statement,
    params: I,
) -> Result<Portal, Error>
where
    P: BorrowToSql,
    I: IntoIterator<Item = P>,
    I::IntoIter: ExactSizeIterator,
{
    // HACIVAT PATCH §1: the body moved to `bind_with_formats`; every result column binary, as
    // before.
    bind_with_formats(client, statement, params, Some(1)).await
}

// HACIVAT PATCH §1 begin: `bind` with the caller's result-column format codes.
pub async fn bind_with_formats<P, I, K>(
    client: &Arc<InnerClient>,
    statement: Statement,
    params: I,
    result_formats: K,
) -> Result<Portal, Error>
where
    P: BorrowToSql,
    I: IntoIterator<Item = P>,
    I::IntoIter: ExactSizeIterator,
    K: IntoIterator<Item = i16>,
{
    let name = format!("p{}", NEXT_ID.fetch_add(1, Ordering::SeqCst));
    let buf = client.with_buf(|buf| {
        query::encode_bind(&statement, params, &name, result_formats, buf)?;
        // HACIVAT PATCH §1 end
        frontend::sync(buf);
        Ok(buf.split().freeze())
    })?;

    let mut responses = client.send(RequestMessages::Single(FrontendMessage::Raw(buf)))?;

    match responses.next().await? {
        Message::BindComplete => {}
        _ => return Err(Error::unexpected_message()),
    }

    Ok(Portal::new(client, name, statement))
}
