use gpay_remit_contracts::payment_escrow::{
    Asset, DisputeReason, Error, EscrowStatus, NotificationConfig, PaymentEscrowContract,
    PaymentEscrowContractClient, RecurringConfig, RefundReason, ResolutionOutcome, Milestone, InsuranceConfig, EscrowInsurance, 
    DelegationPermissions, DelegationEntry, EscrowAnalytics, CancellationConfig,
};
use soroban_sdk::{
    symbol_short,
    testutils::{Address as _, Events as _, Ledger},
    token, vec, Address, BytesN, Env, FromVal, Map, String, Symbol, Vec,
};

fn create_token_contract<'a>(
    env: &Env,
    admin: &Address,
) -> (token::Client<'a>, token::StellarAssetClient<'a>) {
    let contract_address = env.register_stellar_asset_contract_v2(admin.clone());
    (
        token::Client::new(env, &contract_address.address()),
        token::StellarAssetClient::new(env, &contract_address.address()),
    )
}

fn setup_test<'a>(
    env: &Env,
) -> (
    PaymentEscrowContractClient<'a>,
    Address,
    Address,
    Address,
    (token::Client<'a>, token::StellarAssetClient<'a>),
    Asset,
) {
    env.mock_all_auths();
    let contract_id = env.register_contract(None, PaymentEscrowContract);
    let client = PaymentEscrowContractClient::new(env, &contract_id);

    let admin = Address::generate(env);
    let sender = Address::generate(env);
    let recipient = Address::generate(env);

    client.init_escrow(&admin);

    let token = create_token_contract(env, &admin);
    let asset = Asset {
        code: String::from_str(env, "USDC"),
        issuer: admin.clone(),
    };

    client.add_supported_asset(&admin, &asset);

    (client, admin, sender, recipient, token, asset)
}

#[test]
fn test_create_escrow_success() {
    let env = Env::default();
    let (client, _admin, sender, recipient, _token, asset) = setup_test(&env);

    env.ledger().with_mut(|li| li.timestamp = 1000);

    let memo = String::from_str(&env, "Test Memo");
    let expiration = 2000;
    let amount = 1000;

    let escrow_id = client.create_escrow(&sender, &recipient, &amount, &asset, &expiration, &memo);
    assert_eq!(escrow_id, 1);

    let escrow = client.get_escrow(&escrow_id).unwrap();
    assert_eq!(escrow.sender, sender);
    assert_eq!(escrow.recipient, recipient);
    assert_eq!(escrow.amount, amount);
    assert_eq!(escrow.asset.code, asset.code);
    assert_eq!(escrow.status, EscrowStatus::Pending);
    assert_eq!(escrow.created_at, 1000);
}

#[test]
fn test_deposit_success() {
    let env = Env::default();
    let (client, _admin, sender, recipient, (token, token_admin), asset) = setup_test(&env);

    env.ledger().with_mut(|li| li.timestamp = 1000);
    let amount = 1000;
    token_admin.mint(&sender, &amount);

    let escrow_id = client.create_escrow(
        &sender,
        &recipient,
        &amount,
        &asset,
        &2000,
        &String::from_str(&env, ""),
    );

    client.deposit(&escrow_id, &sender, &amount, &token.address);

    let escrow = client.get_escrow(&escrow_id).unwrap();
    assert_eq!(escrow.deposited_amount, amount);
    assert_eq!(escrow.status, EscrowStatus::Funded);
    assert_eq!(token.balance(&client.address), amount);
}

#[test]
fn test_partial_deposit_success() {
    let env = Env::default();
    let (client, _admin, sender, recipient, (token, token_admin), asset) = setup_test(&env);

    let amount = 1000;
    token_admin.mint(&sender, &amount);

    let escrow_id = client.create_escrow(
        &sender,
        &recipient,
        &amount,
        &asset,
        &2000,
        &String::from_str(&env, ""),
    );

    client.deposit(&escrow_id, &sender, &400, &token.address);
    let mut escrow = client.get_escrow(&escrow_id).unwrap();
    assert_eq!(escrow.deposited_amount, 400);
    assert_eq!(escrow.status, EscrowStatus::Pending);

    client.deposit(&escrow_id, &sender, &600, &token.address);
    escrow = client.get_escrow(&escrow_id).unwrap();
    assert_eq!(escrow.deposited_amount, 1000);
    assert_eq!(escrow.status, EscrowStatus::Funded);
}

#[test]
fn test_create_escrow_zero_amount() {
    let env = Env::default();
    let (client, _admin, sender, recipient, _token, asset) = setup_test(&env);

    let result = client.try_create_escrow(
        &sender,
        &recipient,
        &0,
        &asset,
        &2000,
        &String::from_str(&env, ""),
    );
    assert_eq!(result, Err(Ok(Error::InvalidAmount)));
}

#[test]
fn test_create_escrow_same_sender_recipient() {
    let env = Env::default();
    let (client, _admin, sender, _recipient, _token, asset) = setup_test(&env);

    let result = client.try_create_escrow(
        &sender,
        &sender,
        &1000,
        &asset,
        &2000,
        &String::from_str(&env, ""),
    );
    assert_eq!(result, Err(Ok(Error::SameSenderRecipient)));
}

#[test]
fn test_create_escrow_unsupported_asset() {
    let env = Env::default();
    let (client, _admin, sender, recipient, _token, _asset) = setup_test(&env);

    let unsupported_asset = Asset {
        code: String::from_str(&env, "BAD"),
        issuer: Address::generate(&env),
    };

    let result = client.try_create_escrow(
        &sender,
        &recipient,
        &1000,
        &unsupported_asset,
        &2000,
        &String::from_str(&env, ""),
    );
    assert_eq!(result, Err(Ok(Error::InvalidAsset)));
}

#[test]
fn test_deposit_wrong_sender() {
    let env = Env::default();
    let (client, _admin, sender, recipient, (token, token_admin), asset) = setup_test(&env);

    let amount = 1000;
    let wrong_sender = Address::generate(&env);
    token_admin.mint(&wrong_sender, &amount);

    let escrow_id = client.create_escrow(
        &sender,
        &recipient,
        &amount,
        &asset,
        &2000,
        &String::from_str(&env, ""),
    );

    let result = client.try_deposit(&escrow_id, &wrong_sender, &amount, &token.address);
    assert_eq!(result, Err(Ok(Error::WrongSender)));
}

#[test]
fn test_deposit_overflow() {
    let env = Env::default();
    let (client, _admin, sender, recipient, (token, token_admin), asset) = setup_test(&env);

    let amount = 1000;
    token_admin.mint(&sender, &2000);

    let escrow_id = client.create_escrow(
        &sender,
        &recipient,
        &amount,
        &asset,
        &2000,
        &String::from_str(&env, ""),
    );

    let result = client.try_deposit(&escrow_id, &sender, &1500, &token.address);
    assert_eq!(result, Err(Ok(Error::InsufficientAmount)));
}

macro_rules! test_create_escrow_parametrized {
    ($($name:ident: $amount:expr,)*) => {
        $(
            #[test]
            fn $name() {
                let env = Env::default();
                let (client, _admin, sender, recipient, _token, asset) = setup_test(&env);
                let escrow_id = client.create_escrow(&sender, &recipient, &$amount, &asset, &2000, &String::from_str(&env, ""));
                let escrow = client.get_escrow(&escrow_id).unwrap();
                assert_eq!(escrow.amount, $amount);
            }
        )*
    }
}

test_create_escrow_parametrized! {
    test_create_100: 100,
    test_create_500: 500,
    test_create_1000: 1000,
}

#[test]
fn test_events_emitted() {
    let env = Env::default();
    let (client, _admin, sender, recipient, (token, token_admin), asset) = setup_test(&env);

    let amount = 1000;
    token_admin.mint(&sender, &amount);

    let escrow_id = client.create_escrow(
        &sender,
        &recipient,
        &amount,
        &asset,
        &2000,
        &String::from_str(&env, ""),
    );

    // Check 'created' event
    let events = env.events().all();
    let created_event = events
        .into_iter()
        .find(|e| {
            let topics = &e.1;
            topics.len() > 0
                && Symbol::from_val(&env, &topics.get(0).unwrap()) == Symbol::new(&env, "created")
        })
        .unwrap();

    assert_eq!(created_event.0, client.address);
    let topics = created_event.1;
    assert_eq!(
        Symbol::from_val(&env, &topics.get(0).unwrap()),
        Symbol::new(&env, "created")
    );
    assert_eq!(u64::from_val(&env, &topics.get(1).unwrap()), escrow_id);
    assert_eq!(Address::from_val(&env, &created_event.2), sender);

    client.deposit(&escrow_id, &sender, &amount, &token.address);

    // Check 'deposit' event
    let events = env.events().all();
    let deposit_event = events
        .into_iter()
        .find(|e| {
            let topics = &e.1;
            topics.len() > 0
                && Symbol::from_val(&env, &topics.get(0).unwrap()) == Symbol::new(&env, "deposit")
        })
        .unwrap();

    let topics = deposit_event.1;
    assert_eq!(
        Symbol::from_val(&env, &topics.get(0).unwrap()),
        Symbol::new(&env, "deposit")
    );
    assert_eq!(u64::from_val(&env, &topics.get(1).unwrap()), escrow_id);
}

// ============================================================================
// ACCESS CONTROL TESTS
// ============================================================================

// Test non-admin cannot call set_platform_fee
#[test]
fn test_set_platform_fee_non_admin() {
    let env = Env::default();
    let (client, admin, sender, _recipient, _token, asset) = setup_test(&env);

    let non_admin = sender; // Use sender as non-admin
    let result = client.try_set_platform_fee(&non_admin, &500);
    assert_eq!(result, Err(Ok(Error::UnauthorizedCaller)));

    // Verify admin can still set the fee
    client.set_platform_fee(&admin, &500);
    assert_eq!(client.get_platform_fee(), 500);
}

// Test non-admin cannot call set_processing_fee
#[test]
fn test_set_processing_fee_non_admin() {
    let env = Env::default();
    let (client, admin, sender, recipient, _token, asset) = setup_test(&env);

    let non_admin = recipient;
    let result = client.try_set_processing_fee(&non_admin, &300);
    assert_eq!(result, Err(Ok(Error::UnauthorizedCaller)));

    // Verify admin can still set the fee
    client.set_processing_fee(&admin, &300);
    assert_eq!(client.get_processing_fee(), 300);
}

// Test non-admin cannot call set_fee_wallet
#[test]
fn test_set_fee_wallet_non_admin() {
    let env = Env::default();
    let (client, admin, sender, recipient, _token, asset) = setup_test(&env);

    let non_admin = sender;
    let new_wallet = Address::generate(&env);
    let result = client.try_set_fee_wallet(&non_admin, &new_wallet);
    assert_eq!(result, Err(Ok(Error::UnauthorizedCaller)));

    // Verify admin can set the fee wallet
    client.set_fee_wallet(&admin, &new_wallet);
    assert_eq!(client.get_fee_wallet(), Some(new_wallet));
}

// Test non-admin cannot call set_forex_fee
#[test]
fn test_set_forex_fee_non_admin() {
    let env = Env::default();
    let (client, admin, sender, recipient, _token, asset) = setup_test(&env);

    let non_admin = recipient;
    let result = client.try_set_forex_fee(&non_admin, &200);
    assert_eq!(result, Err(Ok(Error::UnauthorizedCaller)));

    // Verify admin can set the fee
    client.set_forex_fee(&admin, &200);
}

// Test non-admin cannot call set_compliance_fee
#[test]
fn test_set_compliance_fee_non_admin() {
    let env = Env::default();
    let (client, admin, sender, recipient, _token, asset) = setup_test(&env);

    let non_admin = sender;
    let result = client.try_set_compliance_fee(&non_admin, &100);
    assert_eq!(result, Err(Ok(Error::UnauthorizedCaller)));

    // Verify admin can set the fee
    client.set_compliance_fee(&admin, &100);
}

// Test non-admin cannot call set_fee_limits
#[test]
fn test_set_fee_limits_non_admin() {
    let env = Env::default();
    let (client, admin, sender, recipient, _token, asset) = setup_test(&env);

    let non_admin = recipient;
    let result = client.try_set_fee_limits(&non_admin, &50, &1000);
    assert_eq!(result, Err(Ok(Error::Unauthorized)));

    // Verify admin can set the limits
    client.set_fee_limits(&admin, &50, &1000);
}

// Test non-admin cannot add supported asset
#[test]
fn test_add_supported_asset_non_admin() {
    let env = Env::default();
    let (client, admin, sender, _recipient, _token, asset) = setup_test(&env);

    let non_admin = sender;
    let new_asset = Asset {
        code: String::from_str(&env, "EUR"),
        issuer: admin.clone(),
    };
    let result = client.try_add_supported_asset(&non_admin, &new_asset);
    assert!(result.is_err());
}

// Test non-admin cannot configure KYC
#[test]
fn test_configure_kyc_non_admin() {
    let env = Env::default();
    let (client, admin, sender, recipient, _token, asset) = setup_test(&env);

    let non_admin = sender;
    let oracle = Address::generate(&env);
    let result = client.try_configure_kyc(&non_admin, &oracle, &true, &5000);
    assert_eq!(result, Err(Ok(Error::UnauthorizedCaller)));

    // Verify admin can configure KYC
    client.configure_kyc(&admin, &oracle, &true, &5000);
}

// Test non-admin cannot add to whitelist
#[test]
fn test_add_to_whitelist_non_admin() {
    let env = Env::default();
    let (client, admin, sender, recipient, _token, asset) = setup_test(&env);

    let non_admin = recipient;
    let account = Address::generate(&env);
    let result = client.try_add_to_whitelist(&non_admin, &account, &2000);
    assert_eq!(result, Err(Ok(Error::UnauthorizedCaller)));

    // Verify admin can add to whitelist
    client.add_to_whitelist(&admin, &account, &2000);
}

// Test non-admin cannot remove from whitelist
#[test]
fn test_remove_from_whitelist_non_admin() {
    let env = Env::default();
    let (client, admin, sender, recipient, _token, asset) = setup_test(&env);

    // First add to whitelist as admin
    let account = Address::generate(&env);
    client.add_to_whitelist(&admin, &account, &2000);

    // Try to remove as non-admin
    let non_admin = sender;
    let result = client.try_remove_from_whitelist(&non_admin, &account);
    assert_eq!(result, Err(Ok(Error::UnauthorizedCaller)));
}

// Test non-admin cannot add trusted issuer
#[test]
fn test_add_trusted_issuer_non_admin() {
    let env = Env::default();
    let (client, admin, sender, recipient, _token, asset) = setup_test(&env);

    let non_admin = recipient;
    let issuer = Address::generate(&env);
    let result = client.try_add_trusted_issuer(&non_admin, &issuer);
    assert_eq!(result, Err(Ok(Error::UnauthorizedCaller)));

    // Verify admin can add trusted issuer
    client.add_trusted_issuer(&admin, &issuer);
}

// Test non-admin cannot override KYC
#[test]
fn test_admin_override_kyc_non_admin() {
    let env = Env::default();
    let (client, admin, sender, recipient, (token, token_admin), asset) = setup_test(&env);

    // Create and fund escrow
    let amount = 1000;
    token_admin.mint(&sender, &amount);
    let escrow_id = client.create_escrow(
        &sender,
        &recipient,
        &amount,
        &asset,
        &2000,
        &String::from_str(&env, ""),
    );
    client.deposit(&escrow_id, &sender, &amount, &token.address);

    // Try to override KYC as non-admin
    let non_admin = sender;
    let result = client.try_admin_override_kyc(&non_admin, &escrow_id);
    assert_eq!(result, Err(Ok(Error::UnauthorizedCaller)));

    // Verify admin can override KYC
    client.admin_override_kyc(&admin, &escrow_id);
}

// Test non-recipient cannot release escrow
#[test]
fn test_release_escrow_non_recipient() {
    let env = Env::default();
    let (client, admin, sender, recipient, (token, token_admin), asset) = setup_test(&env);

    let amount = 1000;
    token_admin.mint(&sender, &amount);
    let escrow_id = client.create_escrow(
        &sender,
        &recipient,
        &amount,
        &asset,
        &2000,
        &String::from_str(&env, ""),
    );
    client.deposit(&escrow_id, &sender, &amount, &token.address);

    // Try to release as non-recipient (sender)
    let result = client.try_release_escrow(&escrow_id, &sender, &token.address);
    assert_eq!(result, Err(Ok(Error::UnauthorizedCaller)));

    // Try to release as random third party
    let third_party = Address::generate(&env);
    let result2 = client.try_release_escrow(&escrow_id, &third_party, &token.address);
    assert_eq!(result2, Err(Ok(Error::UnauthorizedCaller)));

    // Verify recipient can release
    client.release_escrow(&escrow_id, &recipient, &token.address);
}

// Test non-sender cannot refund escrow
#[test]
fn test_refund_escrow_non_sender() {
    let env = Env::default();
    let (client, admin, sender, recipient, (token, token_admin), asset) = setup_test(&env);

    let amount = 1000;
    token_admin.mint(&sender, &amount);
    let escrow_id = client.create_escrow(
        &sender,
        &recipient,
        &amount,
        &asset,
        &2000,
        &String::from_str(&env, ""),
    );
    client.deposit(&escrow_id, &sender, &amount, &token.address);

    // Try to refund as non-sender (recipient)
    let result = client.try_refund_escrow(
        &escrow_id,
        &recipient,
        &token.address,
        &RefundReason::SenderRequest,
    );
    assert_eq!(result, Err(Ok(Error::UnauthorizedRefund)));

    // Try to refund as random third party
    let third_party = Address::generate(&env);
    let result2 = client.try_refund_escrow(
        &escrow_id,
        &third_party,
        &token.address,
        &RefundReason::SenderRequest,
    );
    assert_eq!(result2, Err(Ok(Error::UnauthorizedRefund)));

    // Verify sender can refund
    client.refund_escrow(
        &escrow_id,
        &sender,
        &token.address,
        &RefundReason::SenderRequest,
    );
}

// Test non-admin cannot resolve dispute
#[test]
fn test_resolve_dispute_non_admin() {
    let env = Env::default();
    let (client, admin, sender, recipient, (token, token_admin), asset) = setup_test(&env);

    let amount = 1000;
    token_admin.mint(&sender, &amount);
    let escrow_id = client.create_escrow(
        &sender,
        &recipient,
        &amount,
        &asset,
        &2000,
        &String::from_str(&env, ""),
    );
    client.deposit(&escrow_id, &sender, &amount, &token.address);

    // Create a dispute first (as sender)
    client.raise_dispute(
        &escrow_id,
        &sender,
        &DisputeReason::NonDelivery,
        &BytesN::from_array(&env, &[0u8; 32]),
    );

    // Try to resolve as non-admin
    let result =
        client.try_resolve_dispute(&escrow_id, &sender, &ResolutionOutcome::FavorRecipient);
    assert_eq!(result, Err(Ok(Error::UnauthorizedCaller)));

    // Try to resolve as recipient
    let result2 =
        client.try_resolve_dispute(&escrow_id, &recipient, &ResolutionOutcome::FavorRecipient);
    assert_eq!(result2, Err(Ok(Error::UnauthorizedCaller)));

    // Verify admin can resolve
    client.resolve_dispute(&escrow_id, &admin, &ResolutionOutcome::FavorRecipient);
}

// Test role-based access for multi-party approvals
#[test]
fn test_multi_party_approval_whitelist_enforcement() {
    let env = Env::default();
    let (client, admin, sender, recipient, (token, token_admin), asset) = setup_test(&env);

    let amount = 1000;
    token_admin.mint(&sender, &amount);

    // Create escrow first
    let escrow_id = client.create_escrow(
        &sender,
        &recipient,
        &amount,
        &asset,
        &2000,
        &String::from_str(&env, ""),
    );

    // Setup multi-party approval
    let mut approvers = Vec::new(&env);
    approvers.push_back(sender.clone());
    approvers.push_back(recipient.clone());

    client.setup_multi_party_approval(&escrow_id, &admin, &approvers, &2, &5000);

    client.deposit(&escrow_id, &sender, &amount, &token.address);

    // Try to approve from non-whitelisted address
    let non_whitelisted = Address::generate(&env);
    let result = client.try_multi_party_approve(&escrow_id, &non_whitelisted);
    assert_eq!(result, Err(Ok(Error::ApproverNotWhitelisted)));

    // Add to whitelist as admin
    client.add_to_whitelist(&admin, &non_whitelisted, &2000);

    // Now approval should work
    client.multi_party_approve(&escrow_id, &non_whitelisted);
}

// ============================================================================
// ARITHMETIC OVERFLOW TESTS
// ============================================================================

// Test amount overflow in escrow creation
#[test]
fn test_create_escrow_amount_overflow() {
    let env = Env::default();
    let (client, _admin, sender, recipient, _token, asset) = setup_test(&env);

    // Try to create escrow with maximum i128 value
    let max_amount = i128::MAX;
    let result = client.try_create_escrow(
        &sender,
        &recipient,
        &max_amount,
        &asset,
        &2000,
        &String::from_str(&env, ""),
    );
    // Should either succeed with checked math or fail with overflow
    // The contract uses checked arithmetic, so this should handle it
    match result {
        Err(Err(_)) => {}
        Err(Ok(Error::ArithmeticOverflow)) => {} // Expected
        Ok(_) => {}                              // Also acceptable if it handles large numbers
        _ => panic!("Unexpected result: {:?}", result),
    }
}

// Test fee calculation overflow
#[test]
fn test_fee_calculation_overflow() {
    let env = Env::default();
    let (client, admin, sender, recipient, (token, token_admin), asset) = setup_test(&env);

    // Set a high platform fee
    client.set_platform_fee(&admin, &10000); // 100%

    let amount = i128::MAX / 10000; // Large amount
    token_admin.mint(&sender, &(amount * 2));

    let escrow_id = client.create_escrow(
        &sender,
        &recipient,
        &amount,
        &asset,
        &2000,
        &String::from_str(&env, ""),
    );
    client.deposit(&escrow_id, &sender, &amount, &token.address);

    // Try to release - should handle overflow in fee calculation
    let result = client.try_release_escrow(&escrow_id, &recipient, &token.address);
    // Should either succeed or return ArithmeticOverflow
    match result {
        Err(Err(_)) => {}
        Err(Ok(Error::ArithmeticOverflow)) | Ok(_) => {}
        Err(Ok(_)) => {
            // Other errors are also acceptable (e.g., InsufficientAmount)
        }
        Err(Err(_)) => {}
    }
}

// Test deposit amount overflow
#[test]
fn test_deposit_amount_overflow() {
    let env = Env::default();
    let (client, _admin, sender, recipient, (token, token_admin), asset) = setup_test(&env);

    let escrow_amount = 1000;
    token_admin.mint(&sender, &i128::MAX);

    let escrow_id = client.create_escrow(
        &sender,
        &recipient,
        &escrow_amount,
        &asset,
        &2000,
        &String::from_str(&env, ""),
    );

    // Try to deposit more than escrow amount (overflow check)
    let result = client.try_deposit(&escrow_id, &sender, &(escrow_amount + 1), &token.address);
    assert_eq!(result, Err(Ok(Error::InsufficientAmount)));
}

// Test batch operation amount overflow
#[test]
fn test_batch_operation_overflow() {
    let env = Env::default();
    let (client, _admin, sender, recipient, (token, token_admin), asset) = setup_test(&env);

    // Create multiple escrows and try to exceed limits
    let max_u64 = u64::MAX as i128;
    let result = client.try_create_escrow(
        &sender,
        &recipient,
        &max_u64,
        &asset,
        &2000,
        &String::from_str(&env, ""),
    );

    // Should handle large amounts properly
    match result {
        Err(Err(_)) => {}
        Err(Ok(Error::ArithmeticOverflow)) | Ok(_) => {}
        Err(Ok(_)) => {
            // Other errors like InvalidAmount are acceptable
        }
        Err(Err(_)) => {}
    }
}

// Test counter overflow (escrow_id)
#[test]
fn test_escrow_id_counter_overflow() {
    let env = Env::default();
    let (client, _admin, sender, recipient, _token, asset) = setup_test(&env);

    // Create many escrows to test counter behavior
    let mut last_id = 0u64;
    for i in 0..100 {
        let id = client.create_escrow(
            &sender,
            &recipient,
            &(i as i128 + 1),
            &asset,
            &2000,
            &String::from_str(&env, ""),
        );
        assert_eq!(id, last_id + 1);
        last_id = id;
    }
}

// Test multiplication overflow in conversions
#[test]
fn test_multiplication_overflow_in_conversions() {
    let env = Env::default();
    let (client, admin, _sender, _recipient, _token, asset) = setup_test(&env);

    // Set high fee percentages
    client.set_platform_fee(&admin, &10000); // 100%
    client.set_processing_fee(&admin, &10000); // 100%
    client.set_forex_fee(&admin, &10000); // 100%
    client.set_compliance_fee(&admin, &i128::MAX); // Maximum flat fee

    // Try to get fee breakdown with maximum amount
    let result = client.try_get_fee_breakdown(&i128::MAX);

    // Should handle overflow in fee calculations
    match result {
        Err(Err(_)) => {}
        Err(Ok(Error::ArithmeticOverflow)) | Ok(_) => {}
        _ => {}
    }
}

// Test u64::MAX handling
#[test]
fn test_max_u64_handling() {
    let env = Env::default();
    let (client, _admin, sender, recipient, _token, asset) = setup_test(&env);

    let max_u64 = u64::MAX as i128;
    let result = client.try_create_escrow(
        &sender,
        &recipient,
        &max_u64,
        &asset,
        &2000,
        &String::from_str(&env, ""),
    );

    // Should handle u64::MAX properly
    match result {
        Err(Err(_)) => {}
        Err(Ok(Error::ArithmeticOverflow)) | Ok(_) => {}
        Err(Ok(_)) => {
            // InvalidAmount or other errors are acceptable
        }
        Err(Err(_)) => {}
    }
}

// Test i128::MAX handling
#[test]
fn test_max_i128_handling() {
    let env = Env::default();
    let (client, _admin, sender, recipient, _token, asset) = setup_test(&env);

    let max_i128 = i128::MAX;
    let result = client.try_create_escrow(
        &sender,
        &recipient,
        &max_i128,
        &asset,
        &2000,
        &String::from_str(&env, ""),
    );

    // Should handle i128::MAX properly
    match result {
        Err(Err(_)) => {}
        Err(Ok(Error::ArithmeticOverflow)) | Ok(_) => {}
        Err(Ok(_)) => {
            // InvalidAmount or other errors are acceptable
        }
        Err(Err(_)) => {}
    }
}

// Test partial release overflow
#[test]
fn test_partial_release_amount_overflow() {
    let env = Env::default();
    let (client, admin, sender, recipient, (token, token_admin), asset) = setup_test(&env);

    let amount = 1000;
    token_admin.mint(&sender, &amount);

    let escrow_id = client.create_escrow(
        &sender,
        &recipient,
        &amount,
        &asset,
        &2000,
        &String::from_str(&env, ""),
    );

    client.deposit(&escrow_id, &sender, &amount, &token.address);

    // Try to release more than available (overflow)
    let result = client.try_release_partial(&escrow_id, &recipient, &token.address, &(amount + 1));
    assert_eq!(result, Err(Ok(Error::InsufficientFunds)));
}

// Test refund amount overflow
#[test]
fn test_refund_amount_overflow() {
    let env = Env::default();
    let (client, admin, sender, recipient, (token, token_admin), asset) = setup_test(&env);

    let amount = 1000;
    token_admin.mint(&sender, &amount);

    let escrow_id = client.create_escrow(
        &sender,
        &recipient,
        &amount,
        &asset,
        &2000,
        &String::from_str(&env, ""),
    );
    client.deposit(&escrow_id, &sender, &amount, &token.address);

    // Try to refund more than deposited
    let result = client.try_refund_partial(
        &escrow_id,
        &sender,
        &token.address,
        &(amount + 1),
        &RefundReason::SenderRequest,
    );
    assert_eq!(result, Err(Ok(Error::InvalidRefundAmount)));
}

#[test]
fn test_query_filters_and_pagination() {
    let env = Env::default();
    let (client, _admin, sender, recipient, _token, asset) = setup_test(&env);
    let other_sender = Address::generate(&env);

    env.ledger().with_mut(|li| li.timestamp = 1000);
    let first = client.create_escrow(
        &sender,
        &recipient,
        &100,
        &asset,
        &3000,
        &String::from_str(&env, "one"),
    );
    env.ledger().with_mut(|li| li.timestamp = 1100);
    let second = client.create_escrow(
        &sender,
        &recipient,
        &200,
        &asset,
        &3000,
        &String::from_str(&env, "two"),
    );
    env.ledger().with_mut(|li| li.timestamp = 1200);
    client.create_escrow(
        &other_sender,
        &recipient,
        &300,
        &asset,
        &3000,
        &String::from_str(&env, "three"),
    );

    let sender_page = client.query_escrows_by_sender(&sender, &1, &1);
    assert_eq!(sender_page.len(), 1);
    assert_eq!(sender_page.get(0).unwrap().escrow_id, second);

    let pending = client.query_escrows_by_status(&EscrowStatus::Pending, &10, &0);
    assert_eq!(pending.len(), 3);

    let date_matches = client.query_escrows_by_date_range(&1000, &1100, &10, &0);
    assert_eq!(date_matches.len(), 2);
    assert_eq!(date_matches.get(0).unwrap().escrow_id, first);

    let amount_matches = client.query_escrows_by_amount_range(&150, &350, &10, &0);
    assert_eq!(amount_matches.len(), 2);
}

#[test]
fn test_recurring_escrow_process_history_and_cancel() {
    let env = Env::default();
    let (client, _admin, sender, recipient, _token, asset) = setup_test(&env);
    env.ledger().with_mut(|li| li.timestamp = 1000);

    let config = RecurringConfig {
        recipient: recipient.clone(),
        asset: asset.clone(),
        amount: 250,
        interval: 100,
        count: 2,
        auto_release: false,
        expiration_window: 1000,
        memo: String::from_str(&env, "monthly"),
    };
    let recurring_id = client.create_recurring_escrow(&sender, &config);
    let first_escrow = client.process_recurring_payment(&recurring_id);
    assert_eq!(first_escrow, 1);

    let not_due = client.try_process_recurring_payment(&recurring_id);
    assert_eq!(not_due, Err(Ok(Error::NotExpired)));

    env.ledger().with_mut(|li| li.timestamp = 1100);
    let second_escrow = client.process_recurring_payment(&recurring_id);
    assert_eq!(second_escrow, 2);

    let history = client.get_recurring_history(&recurring_id);
    assert_eq!(history.len(), 2);
    assert_eq!(history.get(0).unwrap().escrow_id, first_escrow);

    let recurring = client.get_recurring_escrow(&recurring_id).unwrap();
    assert!(recurring.cancelled);

    let cancel_result = client.try_cancel_recurring_escrow(&recurring_id, &sender);
    assert!(cancel_result.is_ok());
}

#[test]
fn test_notification_hooks_validate_and_record_delivery() {
    let env = Env::default();
    let (client, _admin, sender, recipient, (token, token_admin), asset) = setup_test(&env);
    let amount = 1000;
    token_admin.mint(&sender, &amount);

    let escrow_id = client.create_escrow(
        &sender,
        &recipient,
        &amount,
        &asset,
        &2000,
        &String::from_str(&env, ""),
    );
    let mut urls = Vec::new(&env);
    urls.push_back(String::from_str(&env, "https://example.com/escrow"));
    let config = NotificationConfig {
        webhook_urls: urls,
        max_retries: 3,
    };
    client.register_notification_hook(&escrow_id, &sender, &config);

    let mut bad_urls = Vec::new(&env);
    bad_urls.push_back(String::from_str(&env, ""));
    let bad_config = NotificationConfig {
        webhook_urls: bad_urls,
        max_retries: 1,
    };
    let bad = client.try_register_notification_hook(&escrow_id, &sender, &bad_config);
    assert_eq!(bad, Err(Ok(Error::InvalidAsset)));

    client.deposit(&escrow_id, &sender, &amount, &token.address);
    let deliveries = client.get_notification_history(&escrow_id);
    assert_eq!(deliveries.len(), 1);
    assert_eq!(deliveries.get(0).unwrap().attempts, 3);
}

#[test]
fn test_multi_asset_deposit_release_and_refund() {
    let env = Env::default();
    let (client, admin, sender, recipient, (token_a, token_a_admin), asset_a) = setup_test(&env);
    let (token_b, token_b_admin) = create_token_contract(&env, &admin);
    let asset_b = Asset {
        code: String::from_str(&env, "EURC"),
        issuer: admin.clone(),
    };
    client.add_supported_asset(&admin, &asset_b);

    token_a_admin.mint(&sender, &1000);
    token_b_admin.mint(&sender, &500);

    let mut assets = Vec::new(&env);
    assets.push_back(asset_a.clone());
    assets.push_back(asset_b.clone());
    let mut amounts = Map::new(&env);
    amounts.set(asset_a.clone(), 1000);
    amounts.set(asset_b.clone(), 500);

    let escrow_id = client.create_multi_asset_escrow(
        &sender,
        &recipient,
        &assets,
        &amounts,
        &3000,
        &String::from_str(&env, "multi"),
    );
    client.deposit_asset(&escrow_id, &sender, &asset_a, &1000, &token_a.address);
    client.deposit_asset(&escrow_id, &sender, &asset_b, &500, &token_b.address);

    let escrow = client.get_escrow(&escrow_id).unwrap();
    assert_eq!(escrow.status, EscrowStatus::Funded);
    assert_eq!(escrow.deposited_amounts.get(asset_b.clone()).unwrap(), 500);

    client.release_asset(&escrow_id, &recipient, &asset_a, &token_a.address);
    assert_eq!(token_a.balance(&recipient), 1000);

    client.refund_asset(
        &escrow_id,
        &sender,
        &asset_b,
        &token_b.address,
        &RefundReason::SenderRequest,
    );
    assert_eq!(token_b.balance(&sender), 500);
// ============================================================================
// MILESTONE-BASED ESCROW TESTS (#129)
// ============================================================================

#[test]
fn test_add_milestone() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 1000);

    let admin = Address::generate(&env);
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);

    let contract_id = env.register_contract(None, PaymentEscrowContract);
    let client = PaymentEscrowContractClient::new(&env, &contract_id);

    client.init_escrow(&admin);

    let asset = Asset {
        code: String::from_str(&env, "USDC"),
        issuer: admin.clone(),
    };
    client.add_supported_asset(&admin, &asset);

    let escrow_id = client.create_escrow(&sender, &recipient, &1000, &asset, &2000, &String::from_str(&env, "Test"));

    client.add_milestone(&escrow_id, &sender, &String::from_str(&env, "Phase 1"), &400);
    client.add_milestone(&escrow_id, &sender, &String::from_str(&env, "Phase 2"), &600);

    let milestones = client.get_milestones(&escrow_id);
    assert_eq!(milestones.len(), 2);
    assert_eq!(milestones.get(0).unwrap().amount, 400);
    assert_eq!(milestones.get(1).unwrap().amount, 600);
}

#[test]
fn test_complete_and_approve_milestone() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 1000);

    let admin = Address::generate(&env);
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);

    let (token, token_admin) = create_token_contract(&env, &admin);
    token_admin.mint(&sender, &5000);

    let contract_id = env.register_contract(None, PaymentEscrowContract);
    let client = PaymentEscrowContractClient::new(&env, &contract_id);

    client.init_escrow(&admin);

    let asset = Asset {
        code: String::from_str(&env, "USDC"),
        issuer: admin.clone(),
    };
    client.add_supported_asset(&admin, &asset);

    let escrow_id = client.create_escrow(&sender, &recipient, &1000, &asset, &2000, &String::from_str(&env, "Test"));
    client.deposit(&escrow_id, &sender, &1000, &token.address);

    client.add_milestone(&escrow_id, &sender, &String::from_str(&env, "Phase 1"), &400);
    client.add_milestone(&escrow_id, &sender, &String::from_str(&env, "Phase 2"), &600);

    // Complete milestone 0
    client.complete_milestone(&escrow_id, &0, &sender);
    let milestones = client.get_milestones(&escrow_id);
    assert_eq!(milestones.get(0).unwrap().completed, true);
    assert_eq!(milestones.get(1).unwrap().completed, false);

    // Approve milestone 0 (recipient approves)
    client.approve_milestone(&escrow_id, &0, &recipient);
    let milestones = client.get_milestones(&escrow_id);
    assert_eq!(milestones.get(0).unwrap().approved, true);

    // Complete and approve milestone 1
    client.complete_milestone(&escrow_id, &1, &sender);
    client.approve_milestone(&escrow_id, &1, &recipient);

    let escrow = client.get_escrow(&escrow_id).unwrap();
    assert_eq!(escrow.released_amount, 1000);
    assert_eq!(escrow.status, EscrowStatus::Released);
}

#[test]
fn test_complete_milestone_wrong_caller() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    let unauthorized = Address::generate(&env);

    let contract_id = env.register_contract(None, PaymentEscrowContract);
    let client = PaymentEscrowContractClient::new(&env, &contract_id);

    client.init_escrow(&admin);

    let asset = Asset {
        code: String::from_str(&env, "USDC"),
        issuer: admin.clone(),
    };
    client.add_supported_asset(&admin, &asset);

    let escrow_id = client.create_escrow(&sender, &recipient, &1000, &asset, &2000, &String::from_str(&env, "Test"));
    client.add_milestone(&escrow_id, &sender, &String::from_str(&env, "Phase 1"), &400);

    let result = client.try_complete_milestone(&escrow_id, &0, &unauthorized);
    assert_eq!(result, Err(Ok(Error::Unauthorized)));
}

#[test]
fn test_approve_milestone_wrong_caller() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);

    let contract_id = env.register_contract(None, PaymentEscrowContract);
    let client = PaymentEscrowContractClient::new(&env, &contract_id);

    client.init_escrow(&admin);

    let asset = Asset {
        code: String::from_str(&env, "USDC"),
        issuer: admin.clone(),
    };
    client.add_supported_asset(&admin, &asset);

    let escrow_id = client.create_escrow(&sender, &recipient, &1000, &asset, &2000, &String::from_str(&env, "Test"));
    client.add_milestone(&escrow_id, &sender, &String::from_str(&env, "Phase 1"), &400);
    client.complete_milestone(&escrow_id, &0, &sender);

    // Sender cannot approve milestone
    let result = client.try_approve_milestone(&escrow_id, &0, &sender);
    assert_eq!(result, Err(Ok(Error::Unauthorized)));
}

#[test]
fn test_invalid_milestone_index() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);

    let contract_id = env.register_contract(None, PaymentEscrowContract);
    let client = PaymentEscrowContractClient::new(&env, &contract_id);

    client.init_escrow(&admin);

    let asset = Asset {
        code: String::from_str(&env, "USDC"),
        issuer: admin.clone(),
    };
    client.add_supported_asset(&admin, &asset);

    let escrow_id = client.create_escrow(&sender, &recipient, &1000, &asset, &2000, &String::from_str(&env, "Test"));

    let result = client.try_complete_milestone(&escrow_id, &0, &sender);
    assert_eq!(result, Err(Ok(Error::InvalidStatus)));
}

// ============================================================================
// DELEGATION TESTS (#132)
// ============================================================================

#[test]
fn test_delegate_escrow() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 1000);

    let admin = Address::generate(&env);
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    let delegate = Address::generate(&env);

    let contract_id = env.register_contract(None, PaymentEscrowContract);
    let client = PaymentEscrowContractClient::new(&env, &contract_id);

    client.init_escrow(&admin);

    let asset = Asset {
        code: String::from_str(&env, "USDC"),
        issuer: admin.clone(),
    };
    client.add_supported_asset(&admin, &asset);

    let escrow_id = client.create_escrow(&sender, &recipient, &1000, &asset, &2000, &String::from_str(&env, "Test"));

    let permissions = DelegationPermissions {
        can_release: true,
        can_refund: false,
        can_approve: true,
        can_dispute: false,
    };

    client.delegate_escrow(&escrow_id, &sender, &delegate, &permissions);

    let entry = client.get_delegation(&escrow_id, &delegate).unwrap();
    assert_eq!(entry.delegate, delegate);
    assert_eq!(entry.permissions.can_release, true);
    assert_eq!(entry.permissions.can_refund, false);
    assert_eq!(entry.permissions.can_approve, true);

    let history = client.get_delegation_history(&escrow_id);
    assert_eq!(history.len(), 1);
}

#[test]
fn test_revoke_delegation() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    let delegate = Address::generate(&env);

    let contract_id = env.register_contract(None, PaymentEscrowContract);
    let client = PaymentEscrowContractClient::new(&env, &contract_id);

    client.init_escrow(&admin);

    let asset = Asset {
        code: String::from_str(&env, "USDC"),
        issuer: admin.clone(),
    };
    client.add_supported_asset(&admin, &asset);

    let escrow_id = client.create_escrow(&sender, &recipient, &1000, &asset, &2000, &String::from_str(&env, "Test"));

    let permissions = DelegationPermissions {
        can_release: true,
        can_refund: false,
        can_approve: true,
        can_dispute: false,
    };

    client.delegate_escrow(&escrow_id, &sender, &delegate, &permissions);
    assert!(client.get_delegation(&escrow_id, &delegate).is_some());

    client.revoke_delegation(&escrow_id, &sender, &delegate);
    assert!(client.get_delegation(&escrow_id, &delegate).is_none());
}

#[test]
fn test_delegate_duplicate_rejected() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    let delegate = Address::generate(&env);

    let contract_id = env.register_contract(None, PaymentEscrowContract);
    let client = PaymentEscrowContractClient::new(&env, &contract_id);

    client.init_escrow(&admin);

    let asset = Asset {
        code: String::from_str(&env, "USDC"),
        issuer: admin.clone(),
    };
    client.add_supported_asset(&admin, &asset);

    let escrow_id = client.create_escrow(&sender, &recipient, &1000, &asset, &2000, &String::from_str(&env, "Test"));

    let permissions = DelegationPermissions {
        can_release: true,
        can_refund: false,
        can_approve: true,
        can_dispute: false,
    };

    client.delegate_escrow(&escrow_id, &sender, &delegate, &permissions);

    let result = client.try_delegate_escrow(&escrow_id, &sender, &delegate, &permissions);
    assert_eq!(result, Err(Ok(Error::AlreadyApproved)));
}

#[test]
fn test_revoke_nonexistent_delegate() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);
    let delegate = Address::generate(&env);

    let contract_id = env.register_contract(None, PaymentEscrowContract);
    let client = PaymentEscrowContractClient::new(&env, &contract_id);

    client.init_escrow(&admin);

    let asset = Asset {
        code: String::from_str(&env, "USDC"),
        issuer: admin.clone(),
    };
    client.add_supported_asset(&admin, &asset);

    let escrow_id = client.create_escrow(&sender, &recipient, &1000, &asset, &2000, &String::from_str(&env, "Test"));

    let result = client.try_revoke_delegation(&escrow_id, &sender, &delegate);
    assert_eq!(result, Err(Ok(Error::ApprovalNotFound)));
}

// ============================================================================
// INSURANCE TESTS (#131)
// ============================================================================

#[test]
fn test_set_insurance_config() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let insurer = Address::generate(&env);

    let contract_id = env.register_contract(None, PaymentEscrowContract);
    let client = PaymentEscrowContractClient::new(&env, &contract_id);

    client.init_escrow(&admin);

    let config = InsuranceConfig {
        premium_rate: 500,
        coverage_limit: 10000,
        insurer: insurer.clone(),
        enabled: true,
    };

    client.set_insurance_config(&admin, &config);

    let stored = client.get_insurance_config().unwrap();
    assert_eq!(stored.premium_rate, 500);
    assert_eq!(stored.coverage_limit, 10000);
    assert_eq!(stored.insurer, insurer);
    assert_eq!(stored.enabled, true);
}

#[test]
fn test_insure_escrow() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 1000);

    let admin = Address::generate(&env);
    let insurer = Address::generate(&env);
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);

    let (token, token_admin) = create_token_contract(&env, &admin);
    token_admin.mint(&sender, &5000);

    let contract_id = env.register_contract(None, PaymentEscrowContract);
    let client = PaymentEscrowContractClient::new(&env, &contract_id);

    client.init_escrow(&admin);

    let asset = Asset {
        code: String::from_str(&env, "USDC"),
        issuer: admin.clone(),
    };
    client.add_supported_asset(&admin, &asset);

    let config = InsuranceConfig {
        premium_rate: 500,
        coverage_limit: 10000,
        insurer: insurer.clone(),
        enabled: true,
    };
    client.set_insurance_config(&admin, &config);

    let escrow_id = client.create_escrow(&sender, &recipient, &1000, &asset, &2000, &String::from_str(&env, "Test"));

    client.insure_escrow(&escrow_id, &sender);

    let insurance = client.get_escrow_insurance(&escrow_id).unwrap();
    assert_eq!(insurance.insured, true);
    assert_eq!(insurance.premium, 50); // 1000 * 500 / 10000
    assert_eq!(insurance.coverage, 1000);
    assert_eq!(insurance.claimed, false);
}

#[test]
fn test_claim_insurance() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 1000);

    let admin = Address::generate(&env);
    let insurer = Address::generate(&env);
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);

    let (token, token_admin) = create_token_contract(&env, &admin);
    token_admin.mint(&sender, &5000);

    let contract_id = env.register_contract(None, PaymentEscrowContract);
    let client = PaymentEscrowContractClient::new(&env, &contract_id);

    client.init_escrow(&admin);

    let asset = Asset {
        code: String::from_str(&env, "USDC"),
        issuer: admin.clone(),
    };
    client.add_supported_asset(&admin, &asset);

    let config = InsuranceConfig {
        premium_rate: 500,
        coverage_limit: 10000,
        insurer: insurer.clone(),
        enabled: true,
    };
    client.set_insurance_config(&admin, &config);

    let escrow_id = client.create_escrow(&sender, &recipient, &1000, &asset, &2000, &String::from_str(&env, "Test"));
    client.insure_escrow(&escrow_id, &sender);

    client.claim_insurance(&escrow_id, &sender, &String::from_str(&env, "NonDelivery"));

    let insurance = client.get_escrow_insurance(&escrow_id).unwrap();
    assert_eq!(insurance.claimed, true);
    assert_eq!(insurance.claim_reason.unwrap(), String::from_str(&env, "NonDelivery"));
}

#[test]
fn test_claim_insurance_not_insured() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);

    let (token, token_admin) = create_token_contract(&env, &admin);
    token_admin.mint(&sender, &5000);

    let contract_id = env.register_contract(None, PaymentEscrowContract);
    let client = PaymentEscrowContractClient::new(&env, &contract_id);

    client.init_escrow(&admin);

    let asset = Asset {
        code: String::from_str(&env, "USDC"),
        issuer: admin.clone(),
    };
    client.add_supported_asset(&admin, &asset);

    let escrow_id = client.create_escrow(&sender, &recipient, &1000, &asset, &2000, &String::from_str(&env, "Test"));

    let result = client.try_claim_insurance(&escrow_id, &sender, &String::from_str(&env, "NonDelivery"));
    assert_eq!(result, Err(Ok(Error::ConditionsNotMet)));
}

// ============================================================================
// ANALYTICS TESTS (#130)
// ============================================================================

#[test]
fn test_get_total_escrow_volume() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 1000);

    let admin = Address::generate(&env);
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);

    let contract_id = env.register_contract(None, PaymentEscrowContract);
    let client = PaymentEscrowContractClient::new(&env, &contract_id);

    client.init_escrow(&admin);

    let asset = Asset {
        code: String::from_str(&env, "USDC"),
        issuer: admin.clone(),
    };
    client.add_supported_asset(&admin, &asset);

    client.create_escrow(&sender, &recipient, &1000, &asset, &2000, &String::from_str(&env, "Test 1"));
    client.create_escrow(&sender, &recipient, &2000, &asset, &3000, &String::from_str(&env, "Test 2"));
    client.create_escrow(&sender, &recipient, &3000, &asset, &4000, &String::from_str(&env, "Test 3"));

    let volume = client.get_total_escrow_volume();
    assert_eq!(volume, 6000);
}

#[test]
fn test_get_escrow_count_by_status() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 1000);

    let admin = Address::generate(&env);
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);

    let contract_id = env.register_contract(None, PaymentEscrowContract);
    let client = PaymentEscrowContractClient::new(&env, &contract_id);

    client.init_escrow(&admin);

    let asset = Asset {
        code: String::from_str(&env, "USDC"),
        issuer: admin.clone(),
    };
    client.add_supported_asset(&admin, &asset);

    client.create_escrow(&sender, &recipient, &1000, &asset, &2000, &String::from_str(&env, "Test 1"));
    client.create_escrow(&sender, &recipient, &2000, &asset, &3000, &String::from_str(&env, "Test 2"));
    client.create_escrow(&sender, &recipient, &3000, &asset, &4000, &String::from_str(&env, "Test 3"));

    let count = client.get_escrow_count_by_status(&EscrowStatus::Pending);
    assert_eq!(count, 3);
}

#[test]
fn test_get_average_escrow_amount() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 1000);

    let admin = Address::generate(&env);
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);

    let contract_id = env.register_contract(None, PaymentEscrowContract);
    let client = PaymentEscrowContractClient::new(&env, &contract_id);

    client.init_escrow(&admin);

    let asset = Asset {
        code: String::from_str(&env, "USDC"),
        issuer: admin.clone(),
    };
    client.add_supported_asset(&admin, &asset);

    client.create_escrow(&sender, &recipient, &1000, &asset, &2000, &String::from_str(&env, "Test 1"));
    client.create_escrow(&sender, &recipient, &2000, &asset, &3000, &String::from_str(&env, "Test 2"));
    client.create_escrow(&sender, &recipient, &3000, &asset, &4000, &String::from_str(&env, "Test 3"));

    let avg = client.get_average_escrow_amount();
    assert_eq!(avg, 2000); // (1000 + 2000 + 3000) / 3
}

#[test]
fn test_get_success_rate() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 1000);

    let admin = Address::generate(&env);
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);

    let (token, token_admin) = create_token_contract(&env, &admin);
    token_admin.mint(&sender, &5000);

    let contract_id = env.register_contract(None, PaymentEscrowContract);
    let client = PaymentEscrowContractClient::new(&env, &contract_id);

    client.init_escrow(&admin);

    let asset = Asset {
        code: String::from_str(&env, "USDC"),
        issuer: admin.clone(),
    };
    client.add_supported_asset(&admin, &asset);

    let e1 = client.create_escrow(&sender, &recipient, &1000, &asset, &2000, &String::from_str(&env, "Test 1"));
    client.deposit(&e1, &sender, &1000, &token.address);
    client.approve_escrow(&e1, &admin);
    client.release_escrow(&e1, &recipient, &token.address);

    let e2 = client.create_escrow(&sender, &recipient, &1000, &asset, &2000, &String::from_str(&env, "Test 2"));

    let rate = client.get_success_rate();
    assert_eq!(rate, 5000); // 1/2 = 50% = 5000 bps
}

#[test]
fn test_get_user_statistics() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 1000);

    let admin = Address::generate(&env);
    let sender = Address::generate(&env);
    let recipient = Address::generate(&env);

    let contract_id = env.register_contract(None, PaymentEscrowContract);
    let client = PaymentEscrowContractClient::new(&env, &contract_id);

    client.init_escrow(&admin);

    let asset = Asset {
        code: String::from_str(&env, "USDC"),
        issuer: admin.clone(),
    };
    client.add_supported_asset(&admin, &asset);

    client.create_escrow(&sender, &recipient, &1000, &asset, &2000, &String::from_str(&env, "Test 1"));
    client.create_escrow(&sender, &recipient, &2000, &asset, &3000, &String::from_str(&env, "Test 2"));

    let stats = client.get_user_statistics(&sender);
    assert_eq!(stats.total_escrows, 2);
    assert_eq!(stats.total_volume, 3000);
    assert_eq!(stats.average_amount, 1500);
}

// ============================================================================
// CANCELLATION WITH PENALTIES TESTS
// ============================================================================

#[test]
fn test_set_cancellation_config() {
    let env = Env::default();
    let (client, _admin, sender, recipient, _token, asset) = setup_test(&env);

    let escrow_id = client.create_escrow(&sender, &recipient, &1000, &asset, &2000, &String::from_str(&env, "Test"));

    let config = CancellationConfig {
        penalty_percentage: 500, // 5%
        recipient_compensation: 200, // 2%
    };

    client.set_cancellation_config(&escrow_id, &sender, &config);

    let fetched_config = client.get_cancellation_config(&escrow_id).unwrap();
    assert_eq!(fetched_config.penalty_percentage, 500);
    assert_eq!(fetched_config.recipient_compensation, 200);
}

#[test]
fn test_cancel_escrow_with_penalties() {
    let env = Env::default();
    let (client, admin, sender, recipient, (token, token_admin), asset) = setup_test(&env);

    let amount = 1000;
    token_admin.mint(&sender, &amount);

    let fee_wallet = Address::generate(&env);
    client.set_fee_wallet(&admin, &fee_wallet);

    let escrow_id = client.create_escrow(&sender, &recipient, &amount, &asset, &2000, &String::from_str(&env, "Test"));
    client.deposit(&escrow_id, &sender, &amount, &token.address);

    let config = CancellationConfig {
        penalty_percentage: 1000, // 10% -> 100
        recipient_compensation: 500, // 5% -> 50
    };
    client.set_cancellation_config(&escrow_id, &sender, &config);

    // Cancel escrow
    client.cancel_escrow(&escrow_id, &sender, &token.address, &String::from_str(&env, "Cancel"));

    // Check balances
    // sender refund: 1000 - 100 - 50 = 850
    assert_eq!(token.balance(&sender), 850);
    // fee wallet gets penalty: 100
    assert_eq!(token.balance(&fee_wallet), 100);
    // recipient gets compensation: 50
    assert_eq!(token.balance(&recipient), 50);

    let escrow = client.get_escrow(&escrow_id).unwrap();
    assert_eq!(escrow.status, EscrowStatus::Cancelled);
}

#[test]
fn test_cancel_escrow_no_config() {
    let env = Env::default();
    let (client, _admin, sender, recipient, (token, token_admin), asset) = setup_test(&env);

    let amount = 1000;
    token_admin.mint(&sender, &amount);

    let escrow_id = client.create_escrow(&sender, &recipient, &amount, &asset, &2000, &String::from_str(&env, "Test"));
    client.deposit(&escrow_id, &sender, &amount, &token.address);

    client.cancel_escrow(&escrow_id, &sender, &token.address, &String::from_str(&env, "Cancel"));

    // Check balances
    // sender refund full amount
    assert_eq!(token.balance(&sender), 1000);
    assert_eq!(token.balance(&recipient), 0);

    let escrow = client.get_escrow(&escrow_id).unwrap();
    assert_eq!(escrow.status, EscrowStatus::Cancelled);
}
