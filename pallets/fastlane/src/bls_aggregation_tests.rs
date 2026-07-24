// ============================================================
// pallets/fastlane/src/bls_aggregation_tests.rs
//
// §14 — REAL BLS signature aggregation (milestone 1: cryptographic proof).
//
// The spec asks for "signature aggregation (BLS) to reduce storage". This
// module proves genuine BLS aggregation over BLS12-381 (the same TinyBLS381
// engine `sp-core` uses for its experimental BLS support): an N-authority
// committee each sign the SAME payload id, all N signatures collapse into a
// SINGLE aggregate signature, and that one signature verifies against the set
// of public keys — with a fixed storage footprint independent of N.
//
// This is test-scoped (dev-dependency) and does NOT touch the on-chain
// attestation flow, so it cannot destabilise the existing sr25519 pipeline or
// the runtime WASM build. Wiring authorities to actually produce BLS
// signatures on-chain (OCW keystore signing, BLS pubkeys in `Authorities`,
// an aggregate-verify path in `do_attest`/slashing) is the follow-on
// integration milestone.
// ============================================================

use rand::thread_rng;
use w3f_bls::{
	single_pop_aggregator::SignatureAggregatorAssumingPoP, Keypair, Message, SerializableToBytes,
	Signature, Signed, TinyBLS381,
};

// BLS12-381 with signatures in G1 (the compact group) — matches sp-core's
// `bls381` choice, so these keys are compatible with the eventual on-chain type.
type Engine = TinyBLS381;

/// Domain-separation context + the payload id an authority attests to, framed
/// as the BLS message. On-chain this is the same `payload_id` bytes.
fn payload_message(payload_id: &[u8; 32]) -> Message {
	Message::new(b"fastlane-attest", payload_id)
}

/// A committee of `n` authorities each sign the same payload id; the `n`
/// signatures aggregate into one signature that verifies — and the aggregate's
/// size is constant regardless of `n` (that is the storage win).
#[test]
fn bls_committee_aggregate_verifies_and_shrinks_storage() {
	let payload_id = [7u8; 32];
	let message = payload_message(&payload_id);

	let n = 5usize; // e.g. a 5-of-5 threshold committee
	let mut keypairs: sp_std::vec::Vec<Keypair<Engine>> =
		(0..n).map(|_| Keypair::<Engine>::generate(thread_rng())).collect();

	// Each authority signs the identical payload message.
	let sigs: sp_std::vec::Vec<Signature<Engine>> =
		keypairs.iter_mut().map(|k| k.sign(&message)).collect();

	// Collapse all N signatures into ONE aggregate.
	let mut agg = SignatureAggregatorAssumingPoP::<Engine>::new(message.clone());
	for s in &sigs {
		agg.add_signature(s);
	}
	for k in &keypairs {
		agg.add_publickey(&k.public);
	}

	let agg_ref: &SignatureAggregatorAssumingPoP<Engine> = &agg;

	// The single aggregate signature verifies against the whole committee.
	assert!(agg_ref.verify(), "aggregate BLS signature over the committee must verify");

	// Storage reduction: N separate signatures vs one aggregate signature.
	let one_sig_len = sigs[0].to_bytes().len();
	let stored_individually = one_sig_len * n;
	let stored_aggregated = agg_ref.signature().to_bytes().len();

	println!(
		"BLS aggregation: {} signatures x {}B = {}B  ->  1 aggregate = {}B",
		n, one_sig_len, stored_individually, stored_aggregated
	);

	assert!(
		stored_aggregated < stored_individually,
		"aggregate ({stored_aggregated}B) must be smaller than {n} individual sigs ({stored_individually}B)"
	);
	// The aggregate is a single signature, so its size does not grow with n.
	assert_eq!(stored_aggregated, one_sig_len);
}

/// The aggregate must NOT verify if it is checked against a different message —
/// i.e. authorities cannot have their aggregate replayed onto another payload.
#[test]
fn bls_aggregate_rejects_wrong_payload() {
	let signed_over = payload_message(&[1u8; 32]);
	let checked_against = payload_message(&[2u8; 32]);

	let mut k0 = Keypair::<Engine>::generate(thread_rng());
	let mut k1 = Keypair::<Engine>::generate(thread_rng());
	let s0 = k0.sign(&signed_over);
	let s1 = k1.sign(&signed_over);

	// Aggregator is initialised with the WRONG message on purpose.
	let mut agg = SignatureAggregatorAssumingPoP::<Engine>::new(checked_against);
	agg.add_signature(&s0);
	agg.add_signature(&s1);
	agg.add_publickey(&k0.public);
	agg.add_publickey(&k1.public);

	let agg_ref: &SignatureAggregatorAssumingPoP<Engine> = &agg;
	assert!(!agg_ref.verify(), "aggregate over the wrong payload must NOT verify");
}
