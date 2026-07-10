use litesvm::LiteSVM;

#[test]
fn program_loads() {
    let mut svm = LiteSVM::new();
    let bytes = include_bytes!("../../../target/deploy/kassandra_oracles_program.so");
    let program_id = solana_pubkey::Pubkey::new_from_array(kassandra_oracles_program::ID.to_bytes());
    svm.add_program(program_id, bytes).unwrap();
    // Loading without panicking is the assertion.
}
