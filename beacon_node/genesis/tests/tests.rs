#![cfg(test)]
use environment::{Environment, EnvironmentBuilder};
use eth1::{Eth1Endpoint, DEFAULT_CHAIN_ID};
use eth1_test_rig::{AnvilEth1Instance, DelayThenDeposit, Middleware};
use genesis::{Eth1Config, Eth1GenesisService};
use sensitive_url::SensitiveUrl;
use state_processing::is_valid_genesis_state;
use std::sync::Arc;
use std::time::Duration;
use types::{
    test_utils::generate_deterministic_keypair, FixedBytesExtended, Hash256, MinimalEthSpec,
};

pub fn new_env() -> Environment<MinimalEthSpec> {
    EnvironmentBuilder::minimal()
        .multi_threaded_tokio_runtime()
        .expect("should start tokio runtime")
        .test_logger()
        .expect("should start null logger")
        .build()
        .expect("should build env")
}

#[test]
fn basic() {
    let env = new_env();
    let log = env.core_context().log().clone();
    let mut spec = (*env.eth2_config().spec).clone();
    spec.min_genesis_time = 0;
    spec.min_genesis_active_validator_count = 8;
    let spec = Arc::new(spec);

    env.runtime().block_on(async {
        let eth1 = AnvilEth1Instance::new(DEFAULT_CHAIN_ID.into())
            .await
            .expect("should start eth1 environment");
        let deposit_contract = &eth1.deposit_contract;
        let client = eth1.json_rpc_client();

        let now = client
            .get_block_number()
            .await
            .map(|v| v.as_u64())
            .expect("should get block number");

        let service = Eth1GenesisService::new(
            Eth1Config {
                endpoint: Eth1Endpoint::NoAuth(
                    SensitiveUrl::parse(eth1.endpoint().as_str()).unwrap(),
                ),
                deposit_contract_address: deposit_contract.address(),
                deposit_contract_deploy_block: now,
                lowest_cached_block_number: now,
                follow_distance: 0,
                block_cache_truncation: None,
                ..Eth1Config::default()
            },
            log,
            spec.clone(),
        )
        .unwrap();

        // NOTE: this test is sensitive to the response speed of the external web3 server. If
        // you're experiencing failures, try increasing the update_interval.
        let update_interval = Duration::from_millis(500);

        let deposits = (0..spec.min_genesis_active_validator_count + 2)
            .map(|i| {
                deposit_contract.deposit_helper::<MinimalEthSpec>(
                    generate_deterministic_keypair(i as usize),
                    Hash256::from_low_u64_le(i),
                    32_000_000_000,
                )
            })
            .map(|deposit| DelayThenDeposit {
                delay: Duration::from_secs(0),
                deposit,
            })
            .collect::<Vec<_>>();

        let deposit_future = deposit_contract.deposit_multiple(deposits);

        let wait_future = service.wait_for_genesis_state::<MinimalEthSpec>(update_interval);

        let state = futures::try_join!(deposit_future, wait_future)
            .map(|(_, state)| state)
            .expect("should finish waiting for genesis");

        // Note: using anvil these deposits are 1-per-block, therefore we know there should only be
        // the minimum number of validators.
        assert_eq!(
            state.validators().len(),
            spec.min_genesis_active_validator_count as usize,
            "should have expected validator count"
        );

        assert!(state.genesis_time() > 0, "should have some genesis time");

        assert!(
            is_valid_genesis_state(&state, &spec),
            "should be valid genesis state"
        );

        assert!(
            is_valid_genesis_state(&state, &spec),
            "should be valid genesis state"
        );
    });
}
