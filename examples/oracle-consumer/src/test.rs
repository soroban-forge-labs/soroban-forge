use super::*;
use soroban_sdk::{Address, Env, Symbol};

/// A minimal mock oracle exposing the same `lastprice` shape as Reflector,
/// so the consumer contract can be exercised without a real deployed oracle.
#[contract]
pub struct MockOracle;

#[contractimpl]
impl MockOracle {
    pub fn set_price(env: Env, asset: Symbol, price: i128, timestamp: u64) {
        env.storage()
            .instance()
            .set(&asset, &PriceData { price, timestamp });
    }

    pub fn lastprice(env: Env, asset: Symbol) -> Option<PriceData> {
        env.storage().instance().get(&asset)
    }
}

fn setup(env: &Env) -> (OracleConsumerContractClient<'_>, Address, Symbol) {
    let oracle_id = env.register(MockOracle, ());
    let oracle = MockOracleClient::new(env, &oracle_id);
    let asset = Symbol::new(env, "XLM");

    // 14 decimals, price = $0.12345678901234
    oracle.set_price(&asset, &12_345_678_901_234, &1_000_000);

    let consumer_id = env.register(
        OracleConsumerContract,
        (oracle_id.clone(), asset.clone(), 14_u32),
    );

    (
        OracleConsumerContractClient::new(env, &consumer_id),
        oracle_id,
        asset,
    )
}

#[test]
fn get_price_reads_the_mock_oracle() {
    let env = Env::default();
    let (consumer, _oracle_id, _asset) = setup(&env);

    assert_eq!(consumer.get_price(), 12_345_678_901_234);
}

#[test]
fn convert_scales_by_the_oracle_price() {
    let env = Env::default();
    let (consumer, _oracle_id, _asset) = setup(&env);

    // 1 unit (10^14 stroops-equivalent) converts to ~the raw price.
    let amount = 100_000_000_000_000; // 10^14
    let converted = consumer.convert(&amount);

    assert_eq!(converted, 12_345_678_901_234);
}

#[test]
#[should_panic]
fn get_price_panics_when_the_oracle_has_no_data() {
    let env = Env::default();
    let oracle_id = env.register(MockOracle, ());
    let asset = Symbol::new(&env, "XLM");
    // No `set_price` call — the oracle has no data for this asset.

    let consumer_id = env.register(
        OracleConsumerContract,
        (oracle_id, asset, 14_u32),
    );
    let consumer = OracleConsumerContractClient::new(&env, &consumer_id);

    consumer.get_price();
}
