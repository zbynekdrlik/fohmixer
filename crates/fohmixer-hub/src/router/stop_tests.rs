//! The router's stop closes every page it was told of (#246), also one whose
//! `Attach` came after the `Stop`: a page's session sends its `hello`, then
//! its `Attach`, so a stop between the two leaves the `Attach` queued
//! behind the `Stop`.

use std::sync::Arc;

use fohmixer_proto::roles::Role;

use super::RouterMsg;
use super::tests::offline_router;
use crate::outbox::Outbox;

#[tokio::test]
async fn an_attach_queued_behind_the_stop_is_closed() {
    let dir = tempfile::tempdir().unwrap();
    let (router, own, _records) = offline_router(dir.path());
    let tx = router.io.tx.clone();
    let late = Arc::new(Outbox::new());
    tx.send(RouterMsg::Stop).unwrap();
    tx.send(RouterMsg::Attach {
        client: 1,
        outbox: Arc::clone(&late),
        peer: "10.0.0.1".into(),
        lan: true,
        role: Role::Engineer,
    })
    .unwrap();
    router.run(own).await;
    assert!(
        late.is_closed(),
        "the page attached behind the stop stays open"
    );
}
