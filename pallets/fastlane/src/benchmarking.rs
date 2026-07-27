// ============================================================
// pallets/fastlane/src/benchmarking.rs
// Benchmarks for the §14 BLS extrinsics — the ones whose cost was unknown
// (in-runtime BLS12-381 pairing verification). Run:
//   cargo build --release --features runtime-benchmarks -p solochain-template-node
//   ./target/release/solochain-template-node benchmark pallet \
//     --chain dev --pallet pallet_fastlane --extrinsic '*' \
//     --steps 50 --repeat 20 --output pallets/fastlane/src/weights.rs
// ============================================================
#![cfg(feature = "runtime-benchmarks")]

use super::*;
use frame_benchmarking::v2::*;
use frame_support::BoundedVec;
use frame_system::RawOrigin;

#[benchmarks]
mod benchmarks {
	use super::*;

	fn add_authority<T: Config>(who: &T::AccountId) {
		let mut auths = Authorities::<T>::get();
		let _ = auths.try_push(who.clone());
		Authorities::<T>::put(auths);
	}

	#[benchmark]
	fn register_bls_key() {
		let who: T::AccountId = account("auth", 0, 0);
		add_authority::<T>(&who);
		let public_key = Pallet::<T>::bls_public_from_seed(&[7u8; 32]);

		#[extrinsic_call]
		_(RawOrigin::Signed(who.clone()), public_key);

		assert!(AuthorityBlsKeys::<T>::contains_key(&who));
	}

	#[benchmark]
	fn attest_bls() {
		let who: T::AccountId = account("auth", 0, 0);
		add_authority::<T>(&who);

		let seed = [7u8; 32];
		let public_key = Pallet::<T>::bls_public_from_seed(&seed);
		let bounded: BoundedVec<u8, frame_support::traits::ConstU32<128>> =
			BoundedVec::try_from(public_key).unwrap();
		AuthorityBlsKeys::<T>::insert(&who, bounded);
		Threshold::<T>::put(1);

		let payload_id: PayloadId = [3u8; 32];
		let now = frame_system::Pallet::<T>::block_number();
		let payload = Payload::<T> {
			creator: who.clone(),
			created_at: now,
			expiry: now + 1000u32.into(),
			domain: 1,
			payload_hash: [1u8; 32],
			nonce: 0,
		};
		Payloads::<T>::insert(payload_id, payload);
		Statuses::<T>::insert(payload_id, Status::Submitted);

		let signature = Pallet::<T>::bls_sign_with_seed(&seed, &payload_id);

		#[extrinsic_call]
		_(RawOrigin::Signed(who.clone()), payload_id, signature);

		// Threshold 1 → aggregate must be stored.
		assert!(BlsAggregate::<T>::get(payload_id).is_some());
	}
}
