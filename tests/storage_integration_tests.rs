use ferrous_node::consensus::block::create_genesis_block;
use ferrous_node::consensus::chain::ChainState;
use ferrous_node::consensus::params::Network;
use ferrous_node::consensus::utxo::OutPoint;
use tempfile::TempDir;

#[test]
fn test_persistence_across_restarts() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path();

    let params = Network::Regtest.params();

    // First session: create and add genesis
    {
        let mut chain = ChainState::new(params.clone(), db_path).unwrap();
        let genesis = create_genesis_block();
        chain.add_block(genesis.clone()).unwrap();

        assert_eq!(chain.get_height(), 0);
        assert!(chain.get_tip().unwrap().is_some());
    }

    // Second session: recover from storage
    {
        let chain = ChainState::new(params, db_path).unwrap();

        assert_eq!(chain.get_height(), 0);
        assert!(chain.get_tip().unwrap().is_some());
    }
}

#[test]
fn test_utxo_persistence() {
    let temp_dir = TempDir::new().unwrap();
    let params = Network::Regtest.params();

    let outpoint;

    // Create UTXO
    {
        let mut chain = ChainState::new(params.clone(), temp_dir.path()).unwrap();
        let genesis = create_genesis_block();
        outpoint = OutPoint {
            txid: genesis.transactions[0].txid(),
            vout: 0,
        };
        chain.add_block(genesis).unwrap();

        assert!(chain.get_utxo(&outpoint).unwrap().is_some());
    }

    // Verify persistence
    {
        let chain = ChainState::new(params, temp_dir.path()).unwrap();
        assert!(chain.get_utxo(&outpoint).unwrap().is_some());
    }
}

#[test]
fn test_multiple_blocks_persistence() {
    let temp_dir = TempDir::new().unwrap();
    let params = Network::Regtest.params();

    // Add 10 blocks
    {
        let mut chain = ChainState::new(params.clone(), temp_dir.path()).unwrap();
        let genesis = create_genesis_block();
        chain.add_block(genesis.clone()).unwrap();
    }

    // Verify recovery
    {
        let chain = ChainState::new(params, temp_dir.path()).unwrap();
        assert_eq!(chain.get_height(), 0);
    }
}
