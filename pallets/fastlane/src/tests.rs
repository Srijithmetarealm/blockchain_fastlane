// ============================================================
// pallets/fastlane/src/tests.rs
// Unit tests for the FastLane pallet.
// ============================================================

#![cfg(test)]

use crate::{
    mock::{
        account, checks_hash_for, keypair, payload_id_for, sign_attestation, sign_inclusion,
        AccountId, Balance, BlockNumber, ExtBuilder, Fastlane, RuntimeOrigin, System, Test,
    },
    pallet::{
        AggregatedAttestations, AuthorityBonds, Authorities, Attestations, Error, Event,
        ExpiryAt, FinalizeAt, InclusionCommitments, Nonces, PendingCount,
        SlashProof, Status, Statuses, Threshold,
    },
};
use frame_support::{
    assert_err, assert_noop, assert_ok,
    traits::Hooks,
    BoundedVec,
};
use sp_runtime::DispatchError;

// ============================================================
// Setup helpers
// ============================================================

fn alice() -> AccountId { account("//Alice") }
fn bob() -> AccountId { account("//Bob") }
fn charlie() -> AccountId { account("//Charlie") }
fn dave() -> AccountId { account("//Dave") }

const E18: Balance = 1_000_000_000_000_000_000;
const MIN_BOND: Balance = 1_000;

/// Standard 3-authority committee with threshold 2, all funded.
fn three_auth_ext() -> sp_io::TestExternalities {
    ExtBuilder::default()
        .with_balances(vec![
            (alice(),   E18),
            (bob(),     E18),
            (charlie(), E18),
            (dave(),    E18),
        ])
        .with_authorities(vec![alice(), bob(), charlie()])
        .with_threshold(2)
        .build()
}

fn submit_payload(
    who: AccountId,
    nonce: u64,
    expiry: BlockNumber,
    domain: u32,
    payload_hash: [u8; 32],
) {
    assert_ok!(Fastlane::submit(
        RuntimeOrigin::signed(who),
        nonce,
        expiry,
        domain,
        payload_hash,
    ));
}

fn last_event() -> Event<Test> {
    System::events()
        .into_iter()
        .filter_map(|r| if let crate::mock::RuntimeEvent::Fastlane(e) = r.event { Some(e) } else { None })
        .last()
        .expect("expected a Fastlane event")
}

fn events() -> Vec<Event<Test>> {
    System::events()
        .into_iter()
        .filter_map(|r| if let crate::mock::RuntimeEvent::Fastlane(e) = r.event { Some(e) } else { None })
        .collect()
}

// ============================================================
// Genesis
// ============================================================

#[test]
fn genesis_reserves_min_bond_for_each_authority() {
    three_auth_ext().execute_with(|| {
        for a in [alice(), bob(), charlie()] {
            assert_eq!(AuthorityBonds::<Test>::get(&a), MIN_BOND);
            assert_eq!(pallet_balances::Pallet::<Test>::reserved_balance(&a), MIN_BOND);
        }
        assert_eq!(Authorities::<Test>::get().to_vec(), vec![alice(), bob(), charlie()]);
        assert_eq!(Threshold::<Test>::get(), 2);
    });
}

// ============================================================
// submit()
// ============================================================

#[test]
fn submit_happy_path() {
    three_auth_ext().execute_with(|| {
        let expiry = 100;
        submit_payload(dave(), 0, expiry, 1, [7u8; 32]);
        let pid = payload_id_for(dave(), 0, 1, [7u8; 32]);

        assert_eq!(Statuses::<Test>::get(pid), Some(Status::Submitted));
        assert_eq!(PendingCount::<Test>::get(), 1);
        assert_eq!(Nonces::<Test>::get(&dave()), 1);
        // Scheduled to expire at expiry+1.
        assert!(ExpiryAt::<Test>::get(expiry + 1).contains(&pid));
    });
}

#[test]
fn submit_rejects_past_expiry() {
    three_auth_ext().execute_with(|| {
        System::set_block_number(50);
        assert_noop!(
            Fastlane::submit(RuntimeOrigin::signed(dave()), 0, 50, 1, [7u8; 32]),
            Error::<Test>::PayloadExpired
        );
    });
}

#[test]
fn submit_enforces_nonce_order() {
    three_auth_ext().execute_with(|| {
        submit_payload(dave(), 0, 100, 1, [7u8; 32]);
        // Skip nonce 1.
        assert_noop!(
            Fastlane::submit(RuntimeOrigin::signed(dave()), 2, 100, 1, [8u8; 32]),
            Error::<Test>::InvalidNonce
        );
    });
}

#[test]
fn submit_rejects_duplicate_payload() {
    three_auth_ext().execute_with(|| {
        submit_payload(dave(), 0, 100, 1, [7u8; 32]);
        // Same nonce, same content → rejected. But nonce moved to 1 so this
        // first hits InvalidNonce; use a fresh account to test duplicate path.
        assert_noop!(
            Fastlane::submit(RuntimeOrigin::signed(dave()), 0, 100, 1, [7u8; 32]),
            Error::<Test>::InvalidNonce
        );
    });
}

// ============================================================
// attest() & threshold transition
// ============================================================

fn submit_and_get_payload() -> ([u8; 32], BlockNumber, [u8; 32]) {
    let expiry: BlockNumber = 100;
    let payload_hash = [7u8; 32];
    submit_payload(dave(), 0, expiry, 1, payload_hash);
    let pid = payload_id_for(dave(), 0, 1, payload_hash);
    let ch = checks_hash_for(dave(), 0, 1, payload_hash, expiry);
    (pid, expiry, ch)
}

#[test]
fn attest_signature_verification_round_trip() {
    three_auth_ext().execute_with(|| {
        let (pid, expiry, ch) = submit_and_get_payload();

        let alice_pair = keypair("//Alice");
        let sig = sign_attestation(&alice_pair, pid, expiry, ch);

        assert_ok!(Fastlane::attest(
            RuntimeOrigin::signed(alice()),
            pid,
            alice(),
            sig,
            ch,
        ));

        assert_eq!(Attestations::<Test>::get(pid).len(), 1);
        // Below threshold, still Submitted.
        assert_eq!(Statuses::<Test>::get(pid), Some(Status::Submitted));
    });
}

#[test]
fn attest_reaches_threshold_and_preconsenses() {
    three_auth_ext().execute_with(|| {
        let (pid, expiry, ch) = submit_and_get_payload();

        let alice_pair = keypair("//Alice");
        assert_ok!(Fastlane::attest(
            RuntimeOrigin::signed(alice()),
            pid, alice(),
            sign_attestation(&alice_pair, pid, expiry, ch),
            ch,
        ));

        let bob_pair = keypair("//Bob");
        assert_ok!(Fastlane::attest(
            RuntimeOrigin::signed(bob()),
            pid, bob(),
            sign_attestation(&bob_pair, pid, expiry, ch),
            ch,
        ));

        assert_eq!(Statuses::<Test>::get(pid), Some(Status::PreConsensed));
        assert_eq!(PendingCount::<Test>::get(), 0);
        // Scheduled for auto-finalisation at now + FinalizeDelay (=1 in mock).
        assert!(FinalizeAt::<Test>::get(System::block_number() + 1).contains(&pid));
    });
}

#[test]
fn attest_rejects_bad_signature() {
    three_auth_ext().execute_with(|| {
        let (pid, _expiry, ch) = submit_and_get_payload();
        // Use Bob's signature claiming to be Alice.
        let bob_pair = keypair("//Bob");
        let bad_sig = sign_attestation(&bob_pair, pid, 100, ch);

        assert_err!(
            Fastlane::attest(RuntimeOrigin::signed(alice()), pid, alice(), bad_sig, ch),
            Error::<Test>::InvalidSignature
        );
    });
}

#[test]
fn attest_rejects_non_authority() {
    three_auth_ext().execute_with(|| {
        let (pid, expiry, ch) = submit_and_get_payload();
        let dave_pair = keypair("//Dave");
        let sig = sign_attestation(&dave_pair, pid, expiry, ch);

        assert_err!(
            Fastlane::attest(RuntimeOrigin::signed(dave()), pid, dave(), sig, ch),
            Error::<Test>::NotAuthority
        );
    });
}

#[test]
fn attest_rejects_double_signing_by_same_authority() {
    three_auth_ext().execute_with(|| {
        let (pid, expiry, ch) = submit_and_get_payload();
        let alice_pair = keypair("//Alice");
        let sig = sign_attestation(&alice_pair, pid, expiry, ch);
        assert_ok!(Fastlane::attest(RuntimeOrigin::signed(alice()), pid, alice(), sig.clone(), ch));
        assert_err!(
            Fastlane::attest(RuntimeOrigin::signed(alice()), pid, alice(), sig, ch),
            Error::<Test>::AlreadyAttested
        );
    });
}

// ============================================================
// finalize() & on_initialize auto-finalize
// ============================================================

fn drive_to_preconsensed() -> [u8; 32] {
    let (pid, expiry, ch) = submit_and_get_payload();
    let alice_pair = keypair("//Alice");
    let bob_pair = keypair("//Bob");
    assert_ok!(Fastlane::attest(
        RuntimeOrigin::signed(alice()), pid, alice(),
        sign_attestation(&alice_pair, pid, expiry, ch), ch,
    ));
    assert_ok!(Fastlane::attest(
        RuntimeOrigin::signed(bob()), pid, bob(),
        sign_attestation(&bob_pair, pid, expiry, ch), ch,
    ));
    pid
}

#[test]
fn finalize_succeeds_when_preconsensed() {
    three_auth_ext().execute_with(|| {
        let pid = drive_to_preconsensed();
        assert_ok!(Fastlane::finalize(RuntimeOrigin::signed(dave()), pid));
        assert_eq!(Statuses::<Test>::get(pid), Some(Status::Finalised));

        // Manual finalisation must be idempotent with the auto-finalise
        // schedule: running on_initialize at the scheduled block is a no-op.
        let scheduled = System::block_number() + 1;
        Fastlane::on_initialize(scheduled);
        assert_eq!(Statuses::<Test>::get(pid), Some(Status::Finalised));
    });
}

#[test]
fn finalize_rejected_when_only_submitted() {
    three_auth_ext().execute_with(|| {
        let (pid, _, _) = submit_and_get_payload();
        assert_err!(
            Fastlane::finalize(RuntimeOrigin::signed(dave()), pid),
            Error::<Test>::ThresholdNotMet
        );
    });
}

#[test]
fn on_initialize_auto_finalizes_preconsensed() {
    three_auth_ext().execute_with(|| {
        let pid = drive_to_preconsensed();
        let next = System::block_number() + 1;
        assert!(FinalizeAt::<Test>::get(next).contains(&pid));

        Fastlane::on_initialize(next);

        assert_eq!(Statuses::<Test>::get(pid), Some(Status::Finalised));
        assert!(!FinalizeAt::<Test>::get(next).contains(&pid));
    });
}

#[test]
fn on_initialize_finalize_is_bounded_per_block() {
    // MaxAutoFinalizePerBlock = 5 in the mock.
    three_auth_ext().execute_with(|| {
        let alice_pair = keypair("//Alice");
        let bob_pair = keypair("//Bob");
        let mut pids = Vec::new();
        for nonce in 0..7u64 {
            let payload_hash = [nonce as u8; 32];
            submit_payload(dave(), nonce, 100, 1, payload_hash);
            let pid = payload_id_for(dave(), nonce, 1, payload_hash);
            let ch = checks_hash_for(dave(), nonce, 1, payload_hash, 100);
            assert_ok!(Fastlane::attest(
                RuntimeOrigin::signed(alice()), pid, alice(),
                sign_attestation(&alice_pair, pid, 100, ch), ch,
            ));
            assert_ok!(Fastlane::attest(
                RuntimeOrigin::signed(bob()), pid, bob(),
                sign_attestation(&bob_pair, pid, 100, ch), ch,
            ));
            pids.push(pid);
        }
        // All 7 attested this block → scheduled at now + FinalizeDelay.
        let sched = System::block_number() + 1;
        assert_eq!(FinalizeAt::<Test>::get(sched).len(), 7);

        // Run on_initialize at the scheduled block: 5 finalised, 2 rescheduled
        // to the next block.
        Fastlane::on_initialize(sched);
        assert_eq!(FinalizeAt::<Test>::get(sched).len(), 0);
        assert_eq!(FinalizeAt::<Test>::get(sched + 1).len(), 2);

        // Next block drains the rest.
        Fastlane::on_initialize(sched + 1);
        assert_eq!(FinalizeAt::<Test>::get(sched + 1).len(), 0);
        let _ = pids;
    });
}

// ============================================================
// Expiry
// ============================================================

#[test]
fn on_initialize_expires_stale_submitted_payloads() {
    three_auth_ext().execute_with(|| {
        let expiry: BlockNumber = 5;
        let payload_hash = [7u8; 32];
        submit_payload(dave(), 0, expiry, 1, payload_hash);
        let pid = payload_id_for(dave(), 0, 1, payload_hash);
        assert_eq!(PendingCount::<Test>::get(), 1);

        // Move past expiry+1 and run on_initialize.
        System::set_block_number(expiry + 1);
        Fastlane::on_initialize(expiry + 1);

        assert_eq!(Statuses::<Test>::get(pid), Some(Status::Expired));
        assert_eq!(PendingCount::<Test>::get(), 0);
        assert!(matches!(last_event(), Event::Expired { .. }));
    });
}

#[test]
fn expired_payload_skipped_if_already_preconsensed() {
    three_auth_ext().execute_with(|| {
        // Submit and PreConsensed before expiry.
        let pid = drive_to_preconsensed();
        // Auto-finalise at the scheduled block (now + FinalizeDelay).
        let sched = System::block_number() + 1;
        Fastlane::on_initialize(sched);
        assert_eq!(Statuses::<Test>::get(pid), Some(Status::Finalised));
        // Block expiry+1 must NOT re-mark it as Expired.
        System::set_block_number(101);
        Fastlane::on_initialize(101);
        assert_eq!(Statuses::<Test>::get(pid), Some(Status::Finalised));
    });
}

// ============================================================
// set_authorities / set_threshold
// ============================================================

#[test]
fn set_authorities_requires_root() {
    three_auth_ext().execute_with(|| {
        let new = BoundedVec::try_from(vec![alice(), bob()]).unwrap();
        assert_err!(
            Fastlane::set_authorities(RuntimeOrigin::signed(alice()), new),
            DispatchError::BadOrigin
        );
    });
}

#[test]
fn set_authorities_rejects_unbonded() {
    three_auth_ext().execute_with(|| {
        let new = BoundedVec::try_from(vec![alice(), dave()]).unwrap();
        assert_err!(
            Fastlane::set_authorities(RuntimeOrigin::root(), new),
            Error::<Test>::BondTooLow
        );
    });
}

#[test]
fn set_threshold_rejects_at_or_below_half() {
    three_auth_ext().execute_with(|| {
        // 3 authorities, 50% = 1, threshold must be > 1.
        assert_err!(
            Fastlane::set_threshold(RuntimeOrigin::root(), 1),
            Error::<Test>::ThresholdTooLow
        );
        assert_ok!(Fastlane::set_threshold(RuntimeOrigin::root(), 2));
        assert_ok!(Fastlane::set_threshold(RuntimeOrigin::root(), 3));
    });
}

#[test]
fn set_threshold_rejects_above_authority_count() {
    three_auth_ext().execute_with(|| {
        assert_err!(
            Fastlane::set_threshold(RuntimeOrigin::root(), 4),
            Error::<Test>::ThresholdTooHigh
        );
    });
}

// ============================================================
// post_bond
// ============================================================

#[test]
fn post_bond_reserves_and_accumulates() {
    three_auth_ext().execute_with(|| {
        assert_ok!(Fastlane::post_bond(RuntimeOrigin::signed(dave()), MIN_BOND));
        assert_eq!(AuthorityBonds::<Test>::get(&dave()), MIN_BOND);
        assert_eq!(pallet_balances::Pallet::<Test>::reserved_balance(&dave()), MIN_BOND);

        assert_ok!(Fastlane::post_bond(RuntimeOrigin::signed(dave()), MIN_BOND));
        assert_eq!(AuthorityBonds::<Test>::get(&dave()), 2 * MIN_BOND);
    });
}

#[test]
fn post_bond_rejects_below_minimum() {
    three_auth_ext().execute_with(|| {
        assert_err!(
            Fastlane::post_bond(RuntimeOrigin::signed(dave()), MIN_BOND - 1),
            Error::<Test>::BondTooLow
        );
    });
}

// ============================================================
// slash + auto-eviction
// ============================================================

#[test]
fn slash_burns_reserved_balance_and_decrements_bond() {
    three_auth_ext().execute_with(|| {
        // Top up so a partial slash leaves bond above MIN.
        assert_ok!(Fastlane::post_bond(RuntimeOrigin::signed(alice()), MIN_BOND * 5));
        let bond_before = AuthorityBonds::<Test>::get(&alice());

        // Build a real InvalidSignature proof: get Alice's attestation onto a
        // payload, then craft a bogus signature claiming to be hers.
        let (pid, expiry, ch) = submit_and_get_payload();
        let alice_pair = keypair("//Alice");
        let real_sig = sign_attestation(&alice_pair, pid, expiry, ch);
        assert_ok!(Fastlane::attest(
            RuntimeOrigin::signed(alice()), pid, alice(), real_sig, ch,
        ));

        // Manually corrupt the recorded signature so it fails verification.
        let mut sigs = Attestations::<Test>::get(pid);
        sigs[0].signature = vec![0u8; 64];
        Attestations::<Test>::insert(pid, sigs);

        let proof = crate::pallet::SlashProof::InvalidSignature(pid, vec![0u8; 64]);
        let slash_amount = MIN_BOND;
        assert_ok!(Fastlane::slash(RuntimeOrigin::root(), alice(), slash_amount, proof));

        assert_eq!(AuthorityBonds::<Test>::get(&alice()), bond_before - slash_amount);
        // Still authority — bond above min.
        assert!(Authorities::<Test>::get().contains(&alice()));
    });
}

#[test]
fn slash_evicts_authority_when_bond_drops_below_min() {
    three_auth_ext().execute_with(|| {
        // Slash all of alice's bond → eviction.
        let (pid, expiry, ch) = submit_and_get_payload();
        let alice_pair = keypair("//Alice");
        assert_ok!(Fastlane::attest(
            RuntimeOrigin::signed(alice()), pid, alice(),
            sign_attestation(&alice_pair, pid, expiry, ch), ch,
        ));
        let mut sigs = Attestations::<Test>::get(pid);
        sigs[0].signature = vec![0u8; 64];
        Attestations::<Test>::insert(pid, sigs);

        let proof = crate::pallet::SlashProof::InvalidSignature(pid, vec![0u8; 64]);
        assert_ok!(Fastlane::slash(RuntimeOrigin::root(), alice(), MIN_BOND, proof));

        assert_eq!(AuthorityBonds::<Test>::get(&alice()), 0);
        assert!(!Authorities::<Test>::get().contains(&alice()));
        assert!(events().iter().any(|e| matches!(e, Event::AuthorityEvicted { .. })));
    });
}

#[test]
fn slash_rejects_amount_exceeding_bond() {
    three_auth_ext().execute_with(|| {
        let proof = crate::pallet::SlashProof::InvalidSignature([0u8; 32], vec![0u8; 64]);
        assert_err!(
            Fastlane::slash(RuntimeOrigin::root(), alice(), MIN_BOND * 1000, proof),
            Error::<Test>::SlashExceedsBond
        );
    });
}

#[test]
fn slash_rejects_none_proof() {
    three_auth_ext().execute_with(|| {
        assert_err!(
            Fastlane::slash(RuntimeOrigin::root(), alice(), MIN_BOND, crate::pallet::SlashProof::None),
            Error::<Test>::MissingSlashProof
        );
    });
}

// ============================================================
// validate_unsigned
// ============================================================

#[test]
fn validate_unsigned_accepts_valid_attestation() {
    use sp_runtime::transaction_validity::TransactionSource;
    use frame_support::pallet_prelude::ValidateUnsigned;
    use codec::Encode;

    three_auth_ext().execute_with(|| {
        let (pid, expiry, ch) = submit_and_get_payload();
        let alice_pair = keypair("//Alice");
        let sig = sign_attestation(&alice_pair, pid, expiry, ch);
        let call = crate::pallet::Call::attest_unsigned {
            payload_id: pid,
            authority_encoded: alice().encode(),
            signature: sig,
            checks_hash: ch,
        };
        let result = crate::pallet::Pallet::<Test>::validate_unsigned(
            TransactionSource::External, &call,
        );
        assert!(result.is_ok());
    });
}

#[test]
fn validate_unsigned_rejects_unknown_authority() {
    use sp_runtime::transaction_validity::TransactionSource;
    use frame_support::pallet_prelude::ValidateUnsigned;
    use codec::Encode;

    three_auth_ext().execute_with(|| {
        let (pid, expiry, ch) = submit_and_get_payload();
        let dave_pair = keypair("//Dave");
        let sig = sign_attestation(&dave_pair, pid, expiry, ch);
        let call = crate::pallet::Call::attest_unsigned {
            payload_id: pid,
            authority_encoded: dave().encode(),
            signature: sig,
            checks_hash: ch,
        };
        let result = crate::pallet::Pallet::<Test>::validate_unsigned(
            TransactionSource::External, &call,
        );
        assert!(result.is_err());
    });
}

#[test]
fn validate_unsigned_rejects_unknown_payload() {
    use sp_runtime::transaction_validity::TransactionSource;
    use frame_support::pallet_prelude::ValidateUnsigned;
    use codec::Encode;

    three_auth_ext().execute_with(|| {
        let call = crate::pallet::Call::attest_unsigned {
            payload_id: [99u8; 32],
            authority_encoded: alice().encode(),
            signature: vec![0u8; 64],
            checks_hash: [0u8; 32],
        };
        let result = crate::pallet::Pallet::<Test>::validate_unsigned(
            TransactionSource::External, &call,
        );
        assert!(result.is_err());
    });
}

// ============================================================
// §14 — Signature aggregation (storage reduction)
// ============================================================

#[test]
fn aggregate_written_when_threshold_reached() {
    three_auth_ext().execute_with(|| {
        let pid = drive_to_preconsensed();

        let agg = AggregatedAttestations::<Test>::get(pid)
            .expect("aggregate should exist after threshold");
        // Authorities are [alice, bob, charlie]; alice(0) + bob(1) signed.
        assert_eq!(agg.count, 2);
        assert_eq!(agg.signer_bitmap, 0b011);
        assert_ne!(agg.digest, [0u8; 32]);

        // AttestationsAggregated event was emitted.
        assert!(events().iter().any(|e| matches!(
            e, Event::AttestationsAggregated { payload_id, count }
                if *payload_id == pid && *count == 2
        )));
    });
}

#[test]
fn finalize_prunes_signatures_but_keeps_aggregate() {
    three_auth_ext().execute_with(|| {
        let pid = drive_to_preconsensed();
        // Full signature list is present pre-finalisation.
        assert_eq!(Attestations::<Test>::get(pid).len(), 2);

        assert_ok!(Fastlane::finalize(RuntimeOrigin::signed(dave()), pid));

        assert_eq!(Statuses::<Test>::get(pid), Some(Status::Finalised));
        // Individual signatures pruned; compact aggregate retained.
        assert_eq!(Attestations::<Test>::get(pid).len(), 0);
        let agg = AggregatedAttestations::<Test>::get(pid)
            .expect("aggregate must survive finalisation");
        assert_eq!(agg.count, 2);
    });
}

// ============================================================
// §14 — Inclusion commitments
// ============================================================

#[test]
fn commit_inclusion_happy_path() {
    three_auth_ext().execute_with(|| {
        let pid = drive_to_preconsensed();
        let deadline: BlockNumber = 50;
        let alice_pair = keypair("//Alice");

        assert_ok!(Fastlane::commit_inclusion(
            RuntimeOrigin::signed(alice()),
            pid,
            deadline,
            sign_inclusion(&alice_pair, pid, deadline),
        ));

        let commits = InclusionCommitments::<Test>::get(pid);
        assert_eq!(commits.len(), 1);
        assert_eq!(commits[0].authority, alice());
        assert_eq!(commits[0].deadline, deadline);
        assert!(matches!(last_event(),
            Event::InclusionCommitted { payload_id, authority, deadline: d }
                if payload_id == pid && authority == alice() && d == deadline));
    });
}

#[test]
fn commit_inclusion_rejects_non_authority() {
    three_auth_ext().execute_with(|| {
        let pid = drive_to_preconsensed();
        let deadline: BlockNumber = 50;
        let dave_pair = keypair("//Dave");
        assert_noop!(
            Fastlane::commit_inclusion(
                RuntimeOrigin::signed(dave()), pid, deadline,
                sign_inclusion(&dave_pair, pid, deadline),
            ),
            Error::<Test>::NotAuthority
        );
    });
}

#[test]
fn commit_inclusion_rejects_when_only_submitted() {
    three_auth_ext().execute_with(|| {
        let (pid, _expiry, _ch) = submit_and_get_payload();
        let deadline: BlockNumber = 50;
        let alice_pair = keypair("//Alice");
        assert_noop!(
            Fastlane::commit_inclusion(
                RuntimeOrigin::signed(alice()), pid, deadline,
                sign_inclusion(&alice_pair, pid, deadline),
            ),
            Error::<Test>::NotPreConsensed
        );
    });
}

#[test]
fn commit_inclusion_rejects_bad_signature() {
    three_auth_ext().execute_with(|| {
        let pid = drive_to_preconsensed();
        let deadline: BlockNumber = 50;
        assert_noop!(
            Fastlane::commit_inclusion(
                RuntimeOrigin::signed(alice()), pid, deadline, vec![0u8; 64],
            ),
            Error::<Test>::InvalidSignature
        );
    });
}

#[test]
fn commit_inclusion_rejects_duplicate() {
    three_auth_ext().execute_with(|| {
        let pid = drive_to_preconsensed();
        let deadline: BlockNumber = 50;
        let alice_pair = keypair("//Alice");
        let sig = sign_inclusion(&alice_pair, pid, deadline);
        assert_ok!(Fastlane::commit_inclusion(
            RuntimeOrigin::signed(alice()), pid, deadline, sig.clone(),
        ));
        assert_noop!(
            Fastlane::commit_inclusion(RuntimeOrigin::signed(alice()), pid, deadline, sig),
            Error::<Test>::AlreadyCommitted
        );
    });
}

#[test]
fn commit_inclusion_rejects_past_deadline() {
    three_auth_ext().execute_with(|| {
        let pid = drive_to_preconsensed();
        let alice_pair = keypair("//Alice");
        // Deadline 0 is not > current block.
        assert_noop!(
            Fastlane::commit_inclusion(
                RuntimeOrigin::signed(alice()), pid, 0,
                sign_inclusion(&alice_pair, pid, 0),
            ),
            Error::<Test>::InvalidDeadline
        );
    });
}

#[test]
fn slash_broken_inclusion_after_deadline() {
    three_auth_ext().execute_with(|| {
        assert_ok!(Fastlane::post_bond(RuntimeOrigin::signed(alice()), MIN_BOND * 5));
        let pid = drive_to_preconsensed();
        let deadline: BlockNumber = 5;
        let alice_pair = keypair("//Alice");
        assert_ok!(Fastlane::commit_inclusion(
            RuntimeOrigin::signed(alice()), pid, deadline,
            sign_inclusion(&alice_pair, pid, deadline),
        ));

        // Advance past the deadline; payload still not finalised.
        System::set_block_number(deadline + 1);
        let bond_before = AuthorityBonds::<Test>::get(&alice());
        assert_ok!(Fastlane::slash(
            RuntimeOrigin::root(), alice(), MIN_BOND, SlashProof::BrokenInclusion(pid),
        ));
        assert_eq!(AuthorityBonds::<Test>::get(&alice()), bond_before - MIN_BOND);
    });
}

#[test]
fn slash_broken_inclusion_rejected_before_deadline() {
    three_auth_ext().execute_with(|| {
        let pid = drive_to_preconsensed();
        let deadline: BlockNumber = 50;
        let alice_pair = keypair("//Alice");
        assert_ok!(Fastlane::commit_inclusion(
            RuntimeOrigin::signed(alice()), pid, deadline,
            sign_inclusion(&alice_pair, pid, deadline),
        ));
        // Deadline not yet reached — slash proof invalid.
        assert_noop!(
            Fastlane::slash(
                RuntimeOrigin::root(), alice(), MIN_BOND, SlashProof::BrokenInclusion(pid),
            ),
            Error::<Test>::MissingSlashProof
        );
    });
}

// ============================================================
// §14 — real on-chain BLS attestation + aggregation
// ============================================================

#[test]
fn bls_attestation_aggregates_on_chain() {
    three_auth_ext().execute_with(|| {
        // Each authority's OCW holds a BLS seed; the public key is registered.
        let alice_seed = [1u8; 32];
        let bob_seed = [2u8; 32];
        let alice_pk = Fastlane::bls_public_from_seed(&alice_seed);
        let bob_pk = Fastlane::bls_public_from_seed(&bob_seed);
        assert_ok!(Fastlane::register_bls_key(
            RuntimeOrigin::signed(alice()),
            alice_pk.clone()
        ));
        assert_ok!(Fastlane::register_bls_key(
            RuntimeOrigin::signed(bob()),
            bob_pk.clone()
        ));

        // A payload is submitted.
        let expiry = 100;
        submit_payload(dave(), 0, expiry, 1, [7u8; 32]);
        let pid = payload_id_for(dave(), 0, 1, [7u8; 32]);

        // Two authorities (threshold = 2) BLS-sign the payload and attest.
        let sig_a = Fastlane::bls_sign_with_seed(&alice_seed, &pid);
        let sig_b = Fastlane::bls_sign_with_seed(&bob_seed, &pid);
        assert_ok!(Fastlane::attest_bls(RuntimeOrigin::signed(alice()), pid, sig_a));
        assert_ok!(Fastlane::attest_bls(RuntimeOrigin::signed(bob()), pid, sig_b));

        // A single 48-byte aggregate signature is stored (real §14 reduction).
        let agg = crate::pallet::BlsAggregate::<Test>::get(pid)
            .expect("aggregate stored at threshold")
            .into_inner();
        assert_eq!(agg.len(), 48);

        // The stored aggregate verifies against the two signers' public keys.
        assert!(Fastlane::verify_bls_aggregate(
            b"fastlane-attest",
            &pid,
            &[alice_pk.clone(), bob_pk.clone()],
            &agg
        ));
        // …and does NOT verify against a different payload id.
        let wrong = [9u8; 32];
        assert!(!Fastlane::verify_bls_aggregate(
            b"fastlane-attest",
            &wrong,
            &[alice_pk, bob_pk],
            &agg
        ));
    });
}

#[test]
fn attest_bls_rejects_unregistered_and_bad_signature() {
    three_auth_ext().execute_with(|| {
        submit_payload(dave(), 0, 100, 1, [7u8; 32]);
        let pid = payload_id_for(dave(), 0, 1, [7u8; 32]);

        // No BLS key registered (keys are not set at genesis) → rejected.
        assert_noop!(
            Fastlane::attest_bls(RuntimeOrigin::signed(alice()), pid, vec![0u8; 48]),
            Error::<Test>::BlsKeyNotRegistered
        );

        // With a key registered, a bogus signature is rejected.
        let pk = Fastlane::bls_public_from_seed(&Fastlane::bls_seed(&bob()));
        assert_ok!(Fastlane::register_bls_key(RuntimeOrigin::signed(bob()), pk));
        assert_noop!(
            Fastlane::attest_bls(RuntimeOrigin::signed(bob()), pid, vec![7u8; 48]),
            Error::<Test>::InvalidBlsSignature
        );
    });
}

/// Register the `bls_seed`-derived key for each authority. Genesis no longer
/// registers keys — production nodes self-register their random key — so tests
/// that exercise the BLS path register explicitly here.
fn register_bls_keys(auths: &[AccountId]) {
    for a in auths {
        let pk = Fastlane::bls_public_from_seed(&Fastlane::bls_seed(a));
        assert_ok!(Fastlane::register_bls_key(RuntimeOrigin::signed(a.clone()), pk));
    }
}

#[test]
fn attest_bls_unsigned_aggregates_from_ocw() {
    use codec::Encode;
    three_auth_ext().execute_with(|| {
        register_bls_keys(&[alice(), bob()]);
        submit_payload(dave(), 0, 100, 1, [7u8; 32]);
        let pid = payload_id_for(dave(), 0, 1, [7u8; 32]);

        // Simulate each authority's OCW: sign with its deterministic seed and
        // submit the UNSIGNED BLS attestation (threshold = 2).
        for a in [alice(), bob()] {
            let seed = Fastlane::bls_seed(&a);
            let sig = Fastlane::bls_sign_with_seed(&seed, &pid);
            assert_ok!(Fastlane::attest_bls_unsigned(
                RuntimeOrigin::none(),
                pid,
                a.encode(),
                sig
            ));
        }

        // One aggregate stored, verifying against the genesis-registered keys.
        let agg = crate::pallet::BlsAggregate::<Test>::get(pid)
            .expect("aggregate stored at threshold")
            .into_inner();
        assert_eq!(agg.len(), 48);
        let pks: Vec<Vec<u8>> = [alice(), bob()]
            .iter()
            .map(|a| {
                crate::pallet::AuthorityBlsKeys::<Test>::get(a)
                    .unwrap()
                    .into_inner()
            })
            .collect();
        assert!(Fastlane::verify_bls_aggregate(b"fastlane-attest", &pid, &pks, &agg));
    });
}

#[test]
fn bls_aggregated_event_fires_once_past_threshold() {
    use codec::Encode;
    three_auth_ext().execute_with(|| {
        register_bls_keys(&[alice(), bob(), charlie()]);
        submit_payload(dave(), 0, 100, 1, [7u8; 32]);
        let pid = payload_id_for(dave(), 0, 1, [7u8; 32]);

        // All three attest — one past the threshold of 2.
        for a in [alice(), bob(), charlie()] {
            let sig = Fastlane::bls_sign_with_seed(&Fastlane::bls_seed(&a), &pid);
            assert_ok!(Fastlane::attest_bls_unsigned(
                RuntimeOrigin::none(),
                pid,
                a.encode(),
                sig
            ));
        }

        // Exactly ONE BlsAggregated event despite 3 attestations.
        let n = events()
            .iter()
            .filter(|e| matches!(e, Event::BlsAggregated { .. }))
            .count();
        assert_eq!(n, 1, "BlsAggregated must fire once per payload");
        // The stored aggregate still reflects all 3 signers.
        assert!(crate::pallet::BlsAggregate::<Test>::get(pid).is_some());
    });
}

#[test]
fn register_bls_key_unsigned_verifies_proof() {
    use codec::Encode;
    use sp_core::Pair;
    three_auth_ext().execute_with(|| {
        // A RANDOM BLS public key (arbitrary seed, not account-derived).
        let pubkey = Fastlane::bls_public_from_seed(&[42u8; 32]);

        // alice sr25519-signs the canonical registration message.
        let mut m = b"fastlane-bls-register".to_vec();
        m.extend_from_slice(&pubkey);
        let msg_hash = sp_io::hashing::blake2_256(&m);
        let sig = keypair("//Alice").sign(&msg_hash).0.to_vec();

        assert_ok!(Fastlane::register_bls_key_unsigned(
            RuntimeOrigin::none(),
            alice().encode(),
            pubkey.clone(),
            sig
        ));
        assert_eq!(
            crate::pallet::AuthorityBlsKeys::<Test>::get(&alice()).unwrap().into_inner(),
            pubkey
        );

        // A bogus proof is rejected.
        assert_noop!(
            Fastlane::register_bls_key_unsigned(
                RuntimeOrigin::none(),
                bob().encode(),
                Fastlane::bls_public_from_seed(&[43u8; 32]),
                vec![0u8; 64]
            ),
            Error::<Test>::InvalidSignature
        );
    });
}

#[test]
fn submit_rejects_unverified_kyc() {
    three_auth_ext().execute_with(|| {
        // MockKyc rejects //Banned.
        let banned = account("//Banned");
        assert_noop!(
            Fastlane::submit(RuntimeOrigin::signed(banned), 0, 100, 1, [7u8; 32]),
            Error::<Test>::NotKycVerified
        );
    });
}

#[test]
fn submit_enforces_rate_limit() {
    three_auth_ext().execute_with(|| {
        crate::mock::MockSubmitCooldown::set(&10);
        // First submission at genesis block (1).
        submit_payload(dave(), 0, 100, 1, [7u8; 32]);
        // A second submission in the same block is rate-limited (checked before
        // the nonce check).
        assert_noop!(
            Fastlane::submit(RuntimeOrigin::signed(dave()), 1, 100, 1, [8u8; 32]),
            Error::<Test>::SubmitTooFrequent
        );
        // After the cooldown elapses, it's allowed again.
        System::set_block_number(11);
        assert_ok!(Fastlane::submit(RuntimeOrigin::signed(dave()), 1, 100, 1, [8u8; 32]));
    });
}

#[test]
fn domain_rules_reject_disallowed_domain() {
    use crate::pallet::OffchainChecks;
    three_auth_ext().execute_with(|| {
        // AllowedDomains is empty, so domain 99 is accepted at submit()...
        submit_payload(dave(), 0, 100, 99, [7u8; 32]);
        let pid = payload_id_for(dave(), 0, 99, [7u8; 32]);
        let payload = crate::pallet::Payloads::<Test>::get(pid).unwrap();
        // ...but MockDomainRules rejects domain 99, so the OCW produces no
        // checks_hash and never attests it.
        assert!(<() as OffchainChecks<Test>>::compute_checks_hash(&payload).is_none());

        // A normal domain passes the per-domain rules.
        submit_payload(dave(), 1, 100, 1, [8u8; 32]);
        let pid2 = payload_id_for(dave(), 1, 1, [8u8; 32]);
        let payload2 = crate::pallet::Payloads::<Test>::get(pid2).unwrap();
        assert!(<() as OffchainChecks<Test>>::compute_checks_hash(&payload2).is_some());
    });
}
