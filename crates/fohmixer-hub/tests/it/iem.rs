//! iemmixer's page (#166): a served hub with `[iem]` tells every page of it
//! when it attaches, a hub without tells none, so the page shows its tab
//! only from the hub's message. No host, no `[tls]`: the hub has no link to
//! iemmixer and serves nothing of it.

use std::time::Duration;

use fohmixer_hub::config::{Config, DEFAULT_IEM_TITLE, IemCfg};
use fohmixer_proto::client::ServerMsg;

use crate::support::TestHub;

const URL: &str = "https://iem.example.org/";

async fn hub(dir: &std::path::Path, iem: Option<IemCfg>) -> TestHub {
    let mut config = Config::defaults(dir);
    config.instances.clear();
    config.iem = iem;
    TestHub::start_config(config).await
}

#[tokio::test]
async fn a_served_hub_with_iem_tells_each_page_its_address_and_title() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = IemCfg {
        url: URL.into(),
        title: DEFAULT_IEM_TITLE.into(),
    };
    let hub = hub(dir.path(), Some(cfg)).await;
    let wanted = ServerMsg::Iem {
        url: URL.into(),
        title: "IEM MIXER".into(),
    };
    for _ in 0..2 {
        let mut page = hub.client().await;
        let told = page
            .wait(Duration::from_secs(5), |msg| {
                matches!(msg, ServerMsg::Iem { .. }).then(|| msg.clone())
            })
            .await;
        assert_eq!(told, wanted);
        page.close().await;
    }
    hub.stop().await;
}

#[tokio::test]
async fn a_served_hub_without_iem_tells_no_page_of_it() {
    let dir = tempfile::tempdir().unwrap();
    let hub = hub(dir.path(), None).await;
    let mut page = hub.client().await;
    // The attach's messages come at once.
    let mut told = Vec::new();
    while let Some(msg) = page.next_message(Duration::from_millis(500)).await {
        told.push(msg);
    }
    let iem = told.iter().any(|m| matches!(m, ServerMsg::Iem { .. }));
    assert!(!iem, "{told:?}");
    page.close().await;
    hub.stop().await;
}
