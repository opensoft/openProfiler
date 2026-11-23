use opensoft_open_profiler_account_status::{
    auth::IdentityConfig,
    model::{self, Grant, Principal, Projection, Registration},
    store::Store,
};
use serde::Deserialize;
use std::{env, fs, time::Duration};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RenewalPolicy {
    company: String,
    accounts: Vec<Registration>,
    grants: Vec<Grant>,
    members: Vec<Principal>,
}

pub fn start_if_configured(store: &Store, identity: &IdentityConfig) -> Result<(), &'static str> {
    let Some(path) = env::var_os("STATUS_BINDING_RENEWAL_FILE") else {
        return Ok(());
    };
    let file = fs::File::open(path).map_err(|_| "binding renewal policy unavailable")?;
    let policy: RenewalPolicy =
        serde_json::from_reader(file).map_err(|_| "invalid binding renewal policy")?;
    let now = model::now();
    let approved = Projection {
        generation: 1,
        expires_at: now + 85_500,
        accounts: policy.accounts,
        grants: policy.grants,
        members: policy.members,
    };
    if !identity.publishers.contains_key(&policy.company) || !approved.valid(now) {
        return Err("invalid company or grants in binding renewal policy");
    }
    let store = store.clone();
    tokio::spawn(async move {
        let mut check = tokio::time::interval(Duration::from_secs(6 * 60 * 60));
        check.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            check.tick().await;
            match store.renew_approved(&policy.company, &approved).await {
                Ok(Some((generation, expires_at))) => {
                    eprintln!(
                        "renewed company bindings {} generation {generation} expires {expires_at}",
                        policy.company
                    );
                }
                Ok(None) => {}
                Err(error) => {
                    eprintln!(
                        "could not renew company bindings {} ({error:?})",
                        policy.company
                    );
                }
            }
        }
    });
    Ok(())
}
