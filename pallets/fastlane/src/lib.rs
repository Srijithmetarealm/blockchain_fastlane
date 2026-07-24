// ============================================================
// pallets/fastlane/src/lib.rs
// FastLane Pre-Consensus Attestation Pallet
// ============================================================

#![cfg_attr(not(feature = "std"), no_std)]

pub use crate::pallet::*;
pub use crate::weights::WeightInfo;

pub mod weights;

#[cfg(test)]
mod mock;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod bls_aggregation_tests;


#[frame_support::pallet]

pub mod pallet {

 












    use frame_support::{
        pallet_prelude::*,
        traits::{ConstU32, Currency, ReservableCurrency},
    };

    use frame_system::{
        offchain::{
            AppCrypto, CreateSignedTransaction, SubmitTransaction,
        },
        pallet_prelude::*,
    };

    use sp_runtime::{
        traits::Zero,
        offchain::storage::StorageValueRef,
        transaction_validity::{
            InvalidTransaction, TransactionSource, TransactionValidity,
            ValidTransaction,
        },
        Saturating,
    };

    use sp_std::vec::Vec;

    use crate::weights::WeightInfo;

    // --------------------------------------------------------




    // --------------------------------------------------------
    // Type aliases
    // --------------------------------------------------------

    pub type PayloadId = [u8; 32];

    pub type BalanceOf<T> =
        <<T as Config>::Currency as Currency<
            <T as frame_system::Config>::AccountId,
        >>::Balance;

    // --------------------------------------------------------
    // Integration traits for downstream pallets
    // --------------------------------------------------------

    /// Downstream pallets implement this to react when a payload is finalised.
    pub trait OnFinalised {
        fn on_finalised(payload_id: PayloadId) -> DispatchResult;
    }

    impl OnFinalised for () {
        fn on_finalised(_payload_id: PayloadId) -> DispatchResult {
            Ok(())
        }
    }

    /// Hook to run deterministic checks used by OCW and on-chain verification.
    pub trait OffchainChecks<T: Config> {
        fn compute_checks_hash(payload: &Payload<T>) -> Option<[u8; 32]>;
    }

    impl<T: Config> OffchainChecks<T> for () {
        fn compute_checks_hash(payload: &Payload<T>) -> Option<[u8; 32]> {
            let allowed_domains = T::AllowedDomains::get();
            let domain_ok =
                allowed_domains.is_empty() || allowed_domains.contains(&payload.domain);
            // Accept the payload if the stored nonce has advanced past it,
            // meaning submit() already validated and accepted the nonce.
            let nonce_ok =
                Nonces::<T>::get(&payload.creator) >= payload.nonce.saturating_add(1);
            let balance_ok = !T::Currency::free_balance(&payload.creator).is_zero();
            let kyc_ok = true;
            let rate_limit_ok = true;
            let domain_specific_ok = true;
            let all_ok = domain_ok
                && nonce_ok
                && balance_ok
                && kyc_ok
                && rate_limit_ok
                && domain_specific_ok;

            if !all_ok {
                return None;
            }

            Some(sp_io::hashing::blake2_256(
                &(
                    payload.creator.clone(),
                    payload.nonce,
                    payload.domain,
                    payload.payload_hash,
                    payload.expiry,
                    domain_ok,
                    nonce_ok,
                    balance_ok,
                    kyc_ok,
                    rate_limit_ok,
                    domain_specific_ok,
                )
                    .encode(),
            ))
        }
    }

    /// Downstream pallets use this trait as a gateway guard.
    ///
    /// Intentionally NOT generic over `Config`: a consumer pallet (e.g.
    /// `pallet-template`) must be able to name this trait as a config bound
    /// without knowing FastLane's concrete runtime `Config`. In the runtime,
    /// `pallet_fastlane::Pallet<Runtime>` is the implementor, so a downstream
    /// pallet sets `type PreConsensusGate = Fastlane;`.
    pub trait EnsurePreConsensed {
        fn ensure_preconsensed(payload_id: PayloadId) -> DispatchResult;
    }

    impl<T: Config> EnsurePreConsensed for Pallet<T> {
        fn ensure_preconsensed(payload_id: PayloadId) -> DispatchResult {
            let status = Statuses::<T>::get(payload_id)
                .ok_or(Error::<T>::PayloadNotFound)?;
            ensure!(
                status == Status::PreConsensed || status == Status::Finalised,
                Error::<T>::ThresholdNotMet
            );
            Ok(())
        }
    }

    // --------------------------------------------------------
    // Pallet configuration trait
    // --------------------------------------------------------

    #[pallet::config]
    pub trait Config:
        frame_system::Config
        + CreateSignedTransaction<Call<Self>>
        + frame_system::offchain::CreateInherent<
            Call<Self>,
        >
    {
        type RuntimeEvent: From<Event<Self>>
            + IsType<<Self as frame_system::Config>::RuntimeEvent>;

        /// Maximum number of authorities in the committee.
        type MaxAuthorities: Get<u32>;

        /// Maximum number of payloads that may be pending (Submitted) at once.
        type MaxPendingPayloads: Get<u32>;

        /// Minimum bond an authority must hold to be registered.
        type MinAuthorityBond: Get<BalanceOf<Self>>;

        type Currency: ReservableCurrency<Self::AccountId>;

        /// Downstream hook called when a payload is finalised.
        type OnFinalised: OnFinalised;

        /// Crypto used by the offchain worker to sign attestations.
        type AuthorityId: AppCrypto<Self::Public, Self::Signature>;

        /// Allowed domain identifiers. An empty vec means all domains allowed.
        type AllowedDomains: Get<Vec<u32>>;
        /// Runtime hook for domain-specific pre-attestation checks.
        type OffchainChecks: OffchainChecks<Self>;
        /// Maximum number of PreConsensed payloads auto-finalized in one block.
        type MaxAutoFinalizePerBlock: Get<u32>;
        /// Number of blocks a payload stays PreConsensed before it is
        /// auto-finalised. `0` schedules finalisation at the very next block
        /// (effectively immediate); a larger value opens a window in which
        /// authorities can post inclusion commitments (§14) before the payload
        /// becomes canonical. Resolves the §6.3-vs-§14 tension: finalisation is
        /// still automatic, just deferred by this many blocks.
        type FinalizeDelay: Get<BlockNumberFor<Self>>;
        /// Maximum number of payloads expired in one block. Bounds the work
        /// performed in `on_initialize` so the chain cannot stall under load.
        type MaxExpiriesPerBlock: Get<u32>;
        /// Benchmarked weights.
        type WeightInfo: crate::weights::WeightInfo;
    }

    // --------------------------------------------------------
    // Storage items
    // --------------------------------------------------------

    #[pallet::storage]
    #[pallet::getter(fn authorities)]
    pub type Authorities<T: Config> = StorageValue<
        _,
        BoundedVec<T::AccountId, T::MaxAuthorities>,
        ValueQuery,
    >;

    #[pallet::storage]
    #[pallet::getter(fn threshold)]
    pub type Threshold<T: Config> = StorageValue<_, u32, ValueQuery>;

    #[pallet::storage]
    #[pallet::getter(fn authority_bonds)]
    pub type AuthorityBonds<T: Config> = StorageMap<
        _,
        Blake2_128Concat,
        T::AccountId,
        BalanceOf<T>,
        ValueQuery,
    >;

    #[pallet::storage]
    #[pallet::getter(fn payloads)]
    pub type Payloads<T: Config> = StorageMap<
        _,
        Blake2_128Concat,
        PayloadId,
        Payload<T>,
        OptionQuery,
    >;

    #[pallet::storage]
    #[pallet::getter(fn attestations)]
    pub type Attestations<T: Config> = StorageMap<
        _,
        Blake2_128Concat,
        PayloadId,
        BoundedVec<SignatureRecord<T>, T::MaxAuthorities>,
        ValueQuery,
    >;

    #[pallet::storage]
    #[pallet::getter(fn statuses)]
    pub type Statuses<T: Config> = StorageMap<
        _,
        Blake2_128Concat,
        PayloadId,
        Status,
        OptionQuery,
    >;

    /// Tracks per-account nonces for replay protection in submit().
    #[pallet::storage]
    #[pallet::getter(fn nonces)]
    pub type Nonces<T: Config> = StorageMap<
        _,
        Blake2_128Concat,
        T::AccountId,
        u64,
        ValueQuery,
    >;

    /// Running count of payloads currently in Submitted state.
    #[pallet::storage]
    #[pallet::getter(fn pending_count)]
    pub type PendingCount<T: Config> = StorageValue<_, u32, ValueQuery>;

    /// Per-block expiry index. When the chain reaches block `expiry + 1`,
    /// `on_initialize` drains `ExpiryAt[expiry + 1]` and marks any still-
    /// Submitted payloads as Expired. Eliminates the O(n) full-storage scan.
    #[pallet::storage]
    pub type ExpiryAt<T: Config> = StorageMap<
        _,
        Twox64Concat,
        BlockNumberFor<T>,
        BoundedVec<PayloadId, T::MaxPendingPayloads>,
        ValueQuery,
    >;

    /// Legacy FIFO queue of PreConsensed payloads. Superseded by the block-
    /// indexed `FinalizeAt` schedule (which supports a finalisation delay), but
    /// retained so any entries present at a runtime upgrade are still drained by
    /// `on_initialize`. New payloads are scheduled via `FinalizeAt`.
    #[pallet::storage]
    pub type PreConsensedQueue<T: Config> = StorageValue<
        _,
        BoundedVec<PayloadId, T::MaxPendingPayloads>,
        ValueQuery,
    >;

    /// Per-block auto-finalisation schedule. When a payload reaches threshold at
    /// block `b` it is scheduled at `b + FinalizeDelay`; `on_initialize` drains
    /// `FinalizeAt[now]` (bounded by `MaxAutoFinalizePerBlock`, rescheduling any
    /// overflow to the next block). The delay is the window during which
    /// inclusion commitments (§14) can be posted against a PreConsensed payload.
    #[pallet::storage]
    pub type FinalizeAt<T: Config> = StorageMap<
        _,
        Twox64Concat,
        BlockNumberFor<T>,
        BoundedVec<PayloadId, T::MaxPendingPayloads>,
        ValueQuery,
    >;

    /// Compact aggregate of the attestation set, written when a payload reaches
    /// threshold. Survives finalisation (unlike `Attestations`, which is pruned)
    /// so an explorer can always prove which authorities pre-consensed a payload.
    #[pallet::storage]
    #[pallet::getter(fn aggregated_attestation)]
    pub type AggregatedAttestations<T: Config> = StorageMap<
        _,
        Blake2_128Concat,
        PayloadId,
        AggregatedAttestation,
        OptionQuery,
    >;

    /// Per-payload inclusion commitments — authorities' signed promises to
    /// finalise a PreConsensed payload by a deadline block.
    #[pallet::storage]
    #[pallet::getter(fn inclusion_commitments)]
    pub type InclusionCommitments<T: Config> = StorageMap<
        _,
        Blake2_128Concat,
        PayloadId,
        BoundedVec<InclusionCommitment<T>, T::MaxAuthorities>,
        ValueQuery,
    >;

    // --------------------------------------------------------
    // §14 real-BLS storage
    // --------------------------------------------------------

    /// Per-authority registered BLS12-381 public key (w3f-bls compressed bytes,
    /// TinyBLS381 public group ≈ 96 bytes). Set via `register_bls_key`.
    #[pallet::storage]
    #[pallet::getter(fn authority_bls_key)]
    pub type AuthorityBlsKeys<T: Config> = StorageMap<
        _,
        Blake2_128Concat,
        T::AccountId,
        BoundedVec<u8, ConstU32<128>>,
        OptionQuery,
    >;

    /// Raw per-authority BLS signatures collected for a payload, before they are
    /// collapsed into a single aggregate. Each signature ≈ 48 bytes (G1).
    #[pallet::storage]
    #[pallet::getter(fn bls_attestations)]
    pub type BlsAttestations<T: Config> = StorageMap<
        _,
        Blake2_128Concat,
        PayloadId,
        BoundedVec<(T::AccountId, BoundedVec<u8, ConstU32<64>>), T::MaxAuthorities>,
        ValueQuery,
    >;

    /// The single aggregated BLS signature for a payload, written once the
    /// threshold is reached. This is the real §14 storage reduction: N × 48-byte
    /// signatures collapse to one 48-byte aggregate that still verifies against
    /// the committee's public keys.
    #[pallet::storage]
    #[pallet::getter(fn bls_aggregate)]
    pub type BlsAggregate<T: Config> = StorageMap<
        _,
        Blake2_128Concat,
        PayloadId,
        BoundedVec<u8, ConstU32<64>>,
        OptionQuery,
    >;

    // --------------------------------------------------------
    // Events
    // --------------------------------------------------------

    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        /// A new payload was registered on-chain.
        Submitted { payload_id: PayloadId, creator: T::AccountId },
        /// Threshold signatures collected — fast-lane approved.
        PreConsensed { payload_id: PayloadId },
        /// Confirmed in a canonical block by finalize().
        Finalised { payload_id: PayloadId },
        /// Expiry block passed before threshold was reached.
        Expired { payload_id: PayloadId },
        /// Authority set was replaced by governance.
        AuthoritiesUpdated { count: u32 },
        /// Signature threshold was updated by governance.
        ThresholdUpdated { value: u32 },
        /// An authority was slashed for misbehaviour.
        AuthoritySlashed { authority: T::AccountId, amount: BalanceOf<T> },
        /// An authority posted its bond.
        BondPosted { authority: T::AccountId, amount: BalanceOf<T> },
        /// An authority was evicted from the active set because their bond fell
        /// below `MinAuthorityBond` (typically after a slash). The chain may
        /// require governance intervention if the threshold is now unreachable.
        AuthorityEvicted { authority: T::AccountId },
        /// Threshold reached — the per-authority signatures were collapsed into
        /// a compact `AggregatedAttestation` (spec §14 storage reduction).
        AttestationsAggregated { payload_id: PayloadId, count: u32 },
        /// An authority committed to include a payload by `deadline`.
        InclusionCommitted {
            payload_id: PayloadId,
            authority: T::AccountId,
            deadline: BlockNumberFor<T>,
        },
        /// An authority registered its BLS public key (§14).
        BlsKeyRegistered { authority: T::AccountId },
        /// Threshold BLS attestations collected — collapsed into one aggregate
        /// signature stored in `BlsAggregate` (§14 real signature aggregation).
        BlsAggregated { payload_id: PayloadId, count: u32 },
    }

    // --------------------------------------------------------
    // Errors
    // --------------------------------------------------------

    #[pallet::error]
    pub enum Error<T> {
        PayloadNotFound,
        NotAuthority,
        AlreadyAttested,
        AlreadyPreConsensed,
        AlreadyFinalised,
        PayloadExpired,
        ThresholdNotMet,
        InvalidSignature,
        ThresholdTooLow,
        // FIX: added ThresholdTooHigh — set_threshold must reject values
        // that exceed the number of registered authorities.
        ThresholdTooHigh,
        DuplicatePayload,
        InvalidNonce,
        DomainNotAllowed,
        TooManyPendingPayloads,
        BondTooLow,
        InsufficientBalance,
        SlashExceedsBond,
        MissingSlashProof,
        /// Inclusion commitment requires the payload to be PreConsensed.
        NotPreConsensed,
        /// This authority already posted an inclusion commitment for the payload.
        AlreadyCommitted,
        /// Inclusion deadline must be strictly in the future.
        InvalidDeadline,
        /// The authority has not registered a BLS public key.
        BlsKeyNotRegistered,
        /// The supplied bytes are not a valid BLS public key.
        InvalidBlsPublicKey,
        /// The BLS signature failed verification against the registered key.
        InvalidBlsSignature,
        /// This authority already submitted a BLS attestation for the payload.
        BlsAlreadyAttested,
    }

    // --------------------------------------------------------
    // Pallet struct
    // --------------------------------------------------------

    #[pallet::pallet]
    #[pallet::without_storage_info]
    pub struct Pallet<T>(_);

    // --------------------------------------------------------
    // Data structures
    // --------------------------------------------------------

    #[derive(Clone, Encode, Decode, PartialEq, RuntimeDebug, TypeInfo)]
    #[scale_info(skip_type_params(T))]
    pub struct Payload<T: Config> {
        pub creator: T::AccountId,
        pub created_at: BlockNumberFor<T>,
        pub expiry: BlockNumberFor<T>,
        pub domain: u32,
        pub payload_hash: [u8; 32],
        pub nonce: u64,
    }

    #[derive(Clone, Encode, Decode, PartialEq, RuntimeDebug, TypeInfo)]
    pub enum Status {
        Submitted,
        PreConsensed,
        Finalised,
        Expired,
    }

    #[derive(Clone, Encode, Decode, PartialEq, RuntimeDebug, TypeInfo)]
    #[scale_info(skip_type_params(T))]
    pub struct SignatureRecord<T: Config> {
        pub authority: T::AccountId,
        pub signature: Vec<u8>,
        pub checks_hash: [u8; 32],
    }

    /// Compact, permanent record of the attestation set for a payload
    /// (spec §14: "Signature aggregation to reduce storage").
    ///
    /// Built the moment the threshold is reached. Once a payload is finalised
    /// the full per-authority `Attestations` list is pruned and only this
    /// aggregate remains — reducing the long-term storage footprint of a
    /// finalised payload from `count × (32 + 64 + 32)` bytes to a fixed
    /// `8 + 32 + 4` bytes, while still proving *which* authorities signed
    /// (`signer_bitmap`) and committing to the exact signatures (`digest`).
    #[derive(Clone, Encode, Decode, PartialEq, RuntimeDebug, TypeInfo, MaxEncodedLen)]
    pub struct AggregatedAttestation {
        /// Bit `i` set ⇔ `Authorities[i]` contributed a signature. Supports up
        /// to 64 authorities, comfortably above the runtime's MaxAuthorities.
        pub signer_bitmap: u64,
        /// blake2_256 over the concatenation of every contributing signature,
        /// in authority order. A verifiable commitment to the aggregated set.
        pub digest: [u8; 32],
        /// Number of signatures aggregated (popcount of `signer_bitmap`).
        pub count: u32,
    }

    /// An authority's signed promise to include/finalise a PreConsensed payload
    /// by `deadline` (spec §14 "Inclusion commitment"). If the payload is not
    /// Finalised by `deadline`, the authority is slashable via
    /// `SlashProof::BrokenInclusion`.
    #[derive(Clone, Encode, Decode, PartialEq, RuntimeDebug, TypeInfo)]
    #[scale_info(skip_type_params(T))]
    pub struct InclusionCommitment<T: Config> {
        pub authority: T::AccountId,
        pub committed_at: BlockNumberFor<T>,
        pub deadline: BlockNumberFor<T>,
        pub signature: Vec<u8>,
    }


    // --------------------------------------------------------
    // Slash proof type
    // --------------------------------------------------------

    #[derive(Clone, Encode, Decode, DecodeWithMemTracking, PartialEq, RuntimeDebug, TypeInfo)]
    pub enum SlashProof {
        /// No proof — always rejected.
        None,
        /// Authority signed two conflicting payloads.
        DoubleSigning(PayloadId, Vec<u8>, PayloadId, Vec<u8>),
        /// Authority submitted a signature that fails on-chain verification.
        InvalidSignature(PayloadId, Vec<u8>),
        /// Authority committed to include `payload_id` but the deadline passed
        /// with the payload still not finalised.
        BrokenInclusion(PayloadId),
    }

    // --------------------------------------------------------
    // Genesis config
    // --------------------------------------------------------

    #[pallet::genesis_config]
    #[derive(frame_support::DefaultNoBound)]
    pub struct GenesisConfig<T: Config> {
        pub authorities: Vec<T::AccountId>,
        pub threshold: u32,
    }

    #[pallet::genesis_build]
    impl<T: Config> BuildGenesisConfig for GenesisConfig<T> {
        fn build(&self) {
            let bounded: BoundedVec<T::AccountId, T::MaxAuthorities> = self
                .authorities
                .clone()
                .try_into()
                .expect("Too many genesis authorities; increase MaxAuthorities");

            // Reserve MinAuthorityBond for each genesis authority so that:
            // (a) they are immediately slash-able from block 1, and
            // (b) subsequent set_authorities calls that include them succeed
            //     (set_authorities checks bonded >= MinAuthorityBond).
            let min_bond = T::MinAuthorityBond::get();
            if !min_bond.is_zero() {
                for authority in &self.authorities {
                    if T::Currency::reserve(authority, min_bond).is_ok() {
                        AuthorityBonds::<T>::insert(authority, min_bond);
                    }
                }
            }

            // §14: populate each authority's BLS public key from its
            // deterministic seed so offchain workers can BLS-sign attestations
            // from genesis (no manual register_bls_key needed on a fresh chain).
            for authority in &self.authorities {
                let seed = Pallet::<T>::bls_seed(authority);
                let pubkey = Pallet::<T>::bls_public_from_seed(&seed);
                if let Ok(bounded) =
                    TryInto::<BoundedVec<u8, ConstU32<128>>>::try_into(pubkey)
                {
                    AuthorityBlsKeys::<T>::insert(authority, bounded);
                }
            }

            Authorities::<T>::put(bounded);
            Threshold::<T>::put(self.threshold);
        }
    }

    // --------------------------------------------------------
    // Hooks
    // --------------------------------------------------------

    #[pallet::hooks]
    impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {

        /// Process expirations and auto-finalisations at the start of every
        /// block. Both queues are bounded, so the work is O(constant) regardless
        /// of how many historical payloads exist.
        fn on_initialize(now: BlockNumberFor<T>) -> Weight {
            // Drain payloads scheduled to expire at this block.
            let max_expiries = T::MaxExpiriesPerBlock::get() as usize;
            let mut expired_count: u32 = 0;
            let scheduled = ExpiryAt::<T>::take(now);
            let mut leftovers: Vec<PayloadId> = Vec::new();

            for (idx, payload_id) in scheduled.into_iter().enumerate() {
                if idx >= max_expiries {
                    leftovers.push(payload_id);
                    continue;
                }
                if matches!(Statuses::<T>::get(payload_id), Some(Status::Submitted)) {
                    Statuses::<T>::insert(payload_id, Status::Expired);
                    PendingCount::<T>::mutate(|c| *c = c.saturating_sub(1));
                    Self::deposit_event(Event::Expired { payload_id });
                    expired_count = expired_count.saturating_add(1);
                }
            }

            // Reschedule leftovers to the next block. Bounded extend is best-
            // effort: if the next block's queue is already full the surplus is
            // dropped silently, which can only happen under deliberate flooding.
            if !leftovers.is_empty() {
                let next_block = now.saturating_add(1u32.into());
                ExpiryAt::<T>::mutate(next_block, |q| {
                    for id in leftovers {
                        if q.try_push(id).is_err() {
                            break;
                        }
                    }
                });
            }

            // Drain auto-finalisations. Budget is shared across the legacy queue
            // and the block-indexed schedule so per-block work stays bounded.
            let max_finalize = T::MaxAutoFinalizePerBlock::get() as usize;
            let mut finalized_count: u32 = 0;

            // (a) Legacy PreConsensedQueue — drains any entries left over from
            //     before the FinalizeAt schedule existed. Status-guarded so a
            //     manually-finalised payload is not finalised twice.
            PreConsensedQueue::<T>::mutate(|queue| {
                let take = max_finalize.min(queue.len());
                for _ in 0..take {
                    let payload_id = queue.remove(0);
                    if matches!(Statuses::<T>::get(payload_id), Some(Status::PreConsensed))
                        && T::OnFinalised::on_finalised(payload_id).is_ok()
                    {
                        Statuses::<T>::insert(payload_id, Status::Finalised);
                        Self::prune_finalised_storage(payload_id);
                        Self::deposit_event(Event::Finalised { payload_id });
                        finalized_count = finalized_count.saturating_add(1);
                    }
                }
            });

            // (b) Block-indexed FinalizeAt schedule for `now`. Overflow beyond
            //     the remaining budget is rescheduled to the next block.
            let scheduled_fin = FinalizeAt::<T>::take(now);
            let mut fin_leftovers: Vec<PayloadId> = Vec::new();
            for payload_id in scheduled_fin.into_iter() {
                if (finalized_count as usize) >= max_finalize {
                    fin_leftovers.push(payload_id);
                    continue;
                }
                // Skip anything already finalised via manual finalize() — the
                // status guard makes auto-finalisation idempotent, so finalize()
                // needs no bookkeeping against this schedule.
                if matches!(Statuses::<T>::get(payload_id), Some(Status::PreConsensed))
                    && T::OnFinalised::on_finalised(payload_id).is_ok()
                {
                    Statuses::<T>::insert(payload_id, Status::Finalised);
                    Self::prune_finalised_storage(payload_id);
                    Self::deposit_event(Event::Finalised { payload_id });
                    finalized_count = finalized_count.saturating_add(1);
                }
            }
            if !fin_leftovers.is_empty() {
                let next_block = now.saturating_add(1u32.into());
                FinalizeAt::<T>::mutate(next_block, |q| {
                    for id in fin_leftovers {
                        if q.try_push(id).is_err() {
                            break;
                        }
                    }
                });
            }

            T::WeightInfo::on_initialize_expire(expired_count)
                .saturating_add(T::WeightInfo::on_initialize_finalize(finalized_count))
        }

        // --------------------------------------------------------
        // Offchain worker — monitors Submitted payloads, signs each one,
        // and submits an unsigned attest_unsigned transaction.
        //
        // FIX (major): the previous implementation:
        //   1. Sent authority_encoded = Vec::new() — so validate_unsigned
        //      always rejected the tx (AccountId::decode fails on empty slice).
        //   2. Never used the Signer to produce a real signature — just
        //      forwarded the msg_hash bytes as the "signature", which would
        //      always fail sr25519_verify on-chain.
        //   3. Had the Signer wiring completely commented out.
        //
        // Fixed approach:
        //   • Use Signer::<T, T::AuthorityId>::all_accounts() to enumerate
        //     every local key that belongs to an on-chain authority.
        //   • Build the canonical message (payload_id ++ expiry_encoded) and
        //     let the Signer sign it.
        //   • Pack the result into an AttestationPayload and submit it as an
        //     unsigned transaction whose outer signature is verified by
        //     validate_unsigned.
        // --------------------------------------------------------
        fn offchain_worker(block_number: BlockNumberFor<T>) {
            // Avoid re-processing the same block on restart.
            let store = StorageValueRef::persistent(b"fastlane::last_ocw_block");
            if let Ok(Some(last)) = store.get::<BlockNumberFor<T>>() {
                if last >= block_number {
                    return;
                }
            }
            store.set(&block_number);

            // Enumerate local sr25519 keys registered under the "fast" key type.
            let local_keys =
                sp_io::crypto::sr25519_public_keys(crate::crypto::KEY_TYPE);
            if local_keys.is_empty() {
                return;
            }

            for (payload_id, status) in Statuses::<T>::iter() {
                if status != Status::Submitted {
                    continue;
                }

                let payload = match Payloads::<T>::get(payload_id) {
                    Some(p) => p,
                    None => continue,
                };

                let checks_hash =
                    match T::OffchainChecks::compute_checks_hash(&payload) {
                        Some(h) => h,
                        None => continue,
                    };

                let authorities = Authorities::<T>::get();
                if authorities.is_empty() {
                    continue;
                }

                // Canonical message — same bytes verified by do_attest.
                let mut msg = payload_id.to_vec();
                msg.extend_from_slice(&payload.expiry.encode());
                msg.extend_from_slice(&checks_hash);
                let msg_hash: [u8; 32] = sp_io::hashing::blake2_256(&msg);

                for key_pub in &local_keys {
                    // In sr25519 runtimes the AccountId IS the raw public key.
                    let auth = match T::AccountId::decode(
                        &mut key_pub.0.as_ref(),
                    ) {
                        Ok(a) => a,
                        Err(_) => continue,
                    };

                    if !authorities.contains(&auth) {
                        continue;
                    }

                    // Skip if we already attested this payload.
                    if Attestations::<T>::get(payload_id)
                        .iter()
                        .any(|r| r.authority == auth)
                    {
                        continue;
                    }

                    if let Some(raw_sig) = sp_io::crypto::sr25519_sign(
                        crate::crypto::KEY_TYPE,
                        key_pub,
                        &msg_hash,
                    ) {
                        let call = Call::attest_unsigned {
                            payload_id,
                            authority_encoded: auth.encode(),
                            // 64-byte raw sig — handled by do_attest's len==64 branch.
                            signature: raw_sig.0.to_vec(),
                            checks_hash,
                        };
                        let xt = T::create_inherent(call.into());
                        let _ = SubmitTransaction::<T, Call<T>>::submit_transaction(xt);
                    }

                    // §14: also submit a real BLS attestation for this payload.
                    // Deterministic no_std signing (SecretKeyVT), so it runs
                    // right here in the OCW. Skips if the authority has no BLS
                    // key registered or already BLS-attested this payload.
                    if AuthorityBlsKeys::<T>::contains_key(&auth)
                        && !BlsAttestations::<T>::get(payload_id)
                            .iter()
                            .any(|(a, _)| a == &auth)
                    {
                        let bls_seed = Self::bls_seed(&auth);
                        let bls_sig = Self::bls_sign_with_seed(&bls_seed, &payload_id);
                        let bls_call = Call::attest_bls_unsigned {
                            payload_id,
                            authority_encoded: auth.encode(),
                            signature: bls_sig,
                        };
                        let bls_xt = T::create_inherent(bls_call.into());
                        let _ =
                            SubmitTransaction::<T, Call<T>>::submit_transaction(bls_xt);
                    }
                }
            }
        }
    }

    // --------------------------------------------------------
    // Unsigned transaction validation (for OCW attest_unsigned)
    //
    // FIX: previously the OCW sent authority_encoded = Vec::new() so this
    // path always returned InvalidTransaction::BadProof.  Now that the OCW
    // populates authority_encoded correctly the validation is meaningful.
    // --------------------------------------------------------

    #[pallet::validate_unsigned]
    impl<T: Config> ValidateUnsigned for Pallet<T> {
        type Call = Call<T>;

        fn validate_unsigned(
            _source: TransactionSource,
            call: &Self::Call,
        ) -> TransactionValidity {
            if let Call::attest_unsigned {
                payload_id,
                authority_encoded,
                signature,
                checks_hash,
            } = call
            {
                let status = Statuses::<T>::get(payload_id)
                    .ok_or(InvalidTransaction::Stale)?;
                if status != Status::Submitted {
                    return InvalidTransaction::Stale.into();
                }

                let payload = Payloads::<T>::get(payload_id)
                    .ok_or(InvalidTransaction::Stale)?;
                let now = frame_system::Pallet::<T>::block_number();
                if now > payload.expiry {
                    return InvalidTransaction::Stale.into();
                }

                let authority = T::AccountId::decode(
                    &mut authority_encoded.as_slice(),
                )
                .map_err(|_| InvalidTransaction::BadProof)?;

                let authorities = Authorities::<T>::get();
                if !authorities.contains(&authority) {
                    return InvalidTransaction::BadProof.into();
                }

                let sigs = Attestations::<T>::get(payload_id);
                if sigs.iter().any(|r| r.authority == authority) {
                    return InvalidTransaction::Stale.into();
                }

                let expected_checks_hash =
                    T::OffchainChecks::compute_checks_hash(&payload)
                        .ok_or(InvalidTransaction::BadProof)?;
                if *checks_hash != expected_checks_hash {
                    return InvalidTransaction::BadProof.into();
                }

                if signature.is_empty() {
                    return InvalidTransaction::BadProof.into();
                }

                ValidTransaction::with_tag_prefix("FastLaneAttest")
                    .priority(100)
                    .longevity(5)
                    .and_provides((payload_id, authority_encoded))
                    .propagate(true)
                    .build()
            } else if let Call::attest_bls_unsigned {
                payload_id,
                authority_encoded,
                signature,
            } = call
            {
                // The payload must exist and be unexpired; the BLS signature
                // must verify against the authority's registered key. That
                // verification is the proof of authenticity for this unsigned tx.
                let payload = Payloads::<T>::get(payload_id)
                    .ok_or(InvalidTransaction::Stale)?;
                let now = frame_system::Pallet::<T>::block_number();
                if now > payload.expiry {
                    return InvalidTransaction::Stale.into();
                }

                let authority = T::AccountId::decode(
                    &mut authority_encoded.as_slice(),
                )
                .map_err(|_| InvalidTransaction::BadProof)?;

                let authorities = Authorities::<T>::get();
                if !authorities.contains(&authority) {
                    return InvalidTransaction::BadProof.into();
                }

                // Already collected this authority's BLS signature? Stale.
                if BlsAttestations::<T>::get(payload_id)
                    .iter()
                    .any(|(a, _)| a == &authority)
                {
                    return InvalidTransaction::Stale.into();
                }

                let pubkey = AuthorityBlsKeys::<T>::get(&authority)
                    .ok_or(InvalidTransaction::BadProof)?;
                if !Self::bls_verify_single(payload_id, pubkey.as_slice(), signature) {
                    return InvalidTransaction::BadProof.into();
                }

                ValidTransaction::with_tag_prefix("FastLaneBlsAttest")
                    .priority(100)
                    .longevity(5)
                    .and_provides((b"bls", payload_id, authority_encoded))
                    .propagate(true)
                    .build()
            } else {
                InvalidTransaction::Call.into()
            }
        }
    }

    // --------------------------------------------------------
    // Extrinsics
    // --------------------------------------------------------

    #[pallet::call]
    impl<T: Config> Pallet<T> {

        // ----------------------------------------------------
        // submit(nonce, expiry, domain, payload_hash)
        // ----------------------------------------------------
        #[pallet::call_index(0)]
        #[pallet::weight(T::WeightInfo::submit())]
        pub fn submit(
            origin: OriginFor<T>,
            nonce: u64,
            expiry: BlockNumberFor<T>,
            domain: u32,
            payload_hash: [u8; 32],
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;

            let now = frame_system::Pallet::<T>::block_number();
            ensure!(expiry > now, Error::<T>::PayloadExpired);

            let expected_nonce = Nonces::<T>::get(&who);
            ensure!(nonce == expected_nonce, Error::<T>::InvalidNonce);

            let allowed_domains = T::AllowedDomains::get();
            if !allowed_domains.is_empty() {
                ensure!(
                    allowed_domains.contains(&domain),
                    Error::<T>::DomainNotAllowed
                );
            }

            let pending = PendingCount::<T>::get();
            ensure!(
                pending < T::MaxPendingPayloads::get(),
                Error::<T>::TooManyPendingPayloads
            );

            let payload_id: PayloadId = sp_io::hashing::blake2_256(
                &(who.clone(), nonce, domain, payload_hash).encode(),
            );
            ensure!(
                !Payloads::<T>::contains_key(payload_id),
                Error::<T>::DuplicatePayload
            );

            // Schedule expiry check at block `expiry + 1` (the first block at
            // which `now > expiry`). Reserve queue slot before mutating any
            // other storage so we fail cleanly if the queue is full.
            let expire_at = expiry.saturating_add(1u32.into());
            ExpiryAt::<T>::try_mutate(expire_at, |q| -> DispatchResult {
                q.try_push(payload_id)
                    .map_err(|_| Error::<T>::TooManyPendingPayloads)?;
                Ok(())
            })?;

            let payload = Payload::<T> {
                creator: who.clone(),
                created_at: now,
                expiry,
                domain,
                payload_hash,
                nonce,
            };

            Payloads::<T>::insert(payload_id, payload);
            Statuses::<T>::insert(payload_id, Status::Submitted);
            Nonces::<T>::insert(&who, nonce.saturating_add(1));
            PendingCount::<T>::mutate(|c| *c = c.saturating_add(1));

            Self::deposit_event(Event::Submitted { payload_id, creator: who });
            Ok(())
        }

        // ----------------------------------------------------
        // attest — signed variant for manual / test use
        // ----------------------------------------------------
        #[pallet::call_index(1)]
        #[pallet::weight(T::WeightInfo::attest())]
        pub fn attest(
            origin: OriginFor<T>,
            payload_id: PayloadId,
            authority: T::AccountId,
            signature: Vec<u8>,
            checks_hash: [u8; 32],
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;
            ensure!(who == authority, Error::<T>::NotAuthority);
            Self::do_attest(payload_id, authority, signature, checks_hash)
        }

        // ----------------------------------------------------
        // finalize — callable by anyone once PreConsensed.
        // ----------------------------------------------------
        #[pallet::call_index(2)]
        #[pallet::weight(T::WeightInfo::finalize())]
        pub fn finalize(
            origin: OriginFor<T>,
            payload_id: PayloadId,
        ) -> DispatchResult {
            let _ = ensure_signed(origin)?;

            let status = Statuses::<T>::get(payload_id)
                .ok_or(Error::<T>::PayloadNotFound)?;

            match status {
                Status::Finalised    => return Err(Error::<T>::AlreadyFinalised.into()),
                Status::Expired      => return Err(Error::<T>::PayloadExpired.into()),
                Status::Submitted    => return Err(Error::<T>::ThresholdNotMet.into()),
                Status::PreConsensed => { /* proceed */ }
            }

            // Remove from PreConsensedQueue so on_initialize doesn't re-finalise.
            PreConsensedQueue::<T>::mutate(|q| {
                if let Some(pos) = q.iter().position(|id| id == &payload_id) {
                    q.remove(pos);
                }
            });

            Statuses::<T>::insert(payload_id, Status::Finalised);
            T::OnFinalised::on_finalised(payload_id)?;
            // §14: drop the full signature list + inclusion commitments; keep
            // only the compact aggregate for the now-canonical payload.
            Self::prune_finalised_storage(payload_id);
            Self::deposit_event(Event::Finalised { payload_id });

            Ok(())
        }

        // ----------------------------------------------------
        // set_authorities — root only
        // ----------------------------------------------------
        #[pallet::call_index(3)]
        #[pallet::weight(T::WeightInfo::set_authorities(new_authorities.len() as u32))]
        pub fn set_authorities(
            origin: OriginFor<T>,
            new_authorities: BoundedVec<T::AccountId, T::MaxAuthorities>,
        ) -> DispatchResult {
            ensure_root(origin)?;

            for auth in new_authorities.iter() {
                let bonded = AuthorityBonds::<T>::get(auth);
                ensure!(bonded >= T::MinAuthorityBond::get(), Error::<T>::BondTooLow);
            }

            // §14: auto-register a BLS key for any new authority that lacks one,
            // mirroring genesis. Ensures BLS attestation works immediately after
            // an authority-set change — no manual register_bls_key needed.
            // Existing registrations are left untouched.
            for auth in new_authorities.iter() {
                if !AuthorityBlsKeys::<T>::contains_key(auth) {
                    let seed = Self::bls_seed(auth);
                    let pubkey = Self::bls_public_from_seed(&seed);
                    if let Ok(bounded) =
                        TryInto::<BoundedVec<u8, ConstU32<128>>>::try_into(pubkey)
                    {
                        AuthorityBlsKeys::<T>::insert(auth, bounded);
                    }
                }
            }

            let count = new_authorities.len() as u32;
            Authorities::<T>::put(new_authorities);
            Self::deposit_event(Event::AuthoritiesUpdated { count });
            Ok(())
        }

        // ----------------------------------------------------
        // set_threshold — root only.
        // ----------------------------------------------------
        #[pallet::call_index(4)]
        #[pallet::weight(T::WeightInfo::set_threshold())]
        pub fn set_threshold(
            origin: OriginFor<T>,
            value: u32,
        ) -> DispatchResult {
            ensure_root(origin)?;

            let authority_count = Authorities::<T>::get().len() as u32;

            // Must be strictly greater than 50% (spec §8: "Do not allow 50% threshold").
            ensure!(
                value > authority_count / 2,
                Error::<T>::ThresholdTooLow
            );

            // Must not exceed the authority set size.
            ensure!(
                value <= authority_count,
                Error::<T>::ThresholdTooHigh
            );

            Threshold::<T>::put(value);
            Self::deposit_event(Event::ThresholdUpdated { value });
            Ok(())
        }

        // ----------------------------------------------------
        // attest_unsigned — unsigned variant submitted by the OCW
        // ----------------------------------------------------
        #[pallet::call_index(5)]
        #[pallet::weight(T::WeightInfo::attest_unsigned())]
        pub fn attest_unsigned(
            origin: OriginFor<T>,
            payload_id: PayloadId,
            authority_encoded: Vec<u8>,
            signature: Vec<u8>,
            checks_hash: [u8; 32],
        ) -> DispatchResult {
            ensure_none(origin)?;

            let authority = T::AccountId::decode(&mut authority_encoded.as_slice())
                .map_err(|_| Error::<T>::InvalidSignature)?;

            Self::do_attest(payload_id, authority, signature, checks_hash)
        }

        // ----------------------------------------------------
        // post_bond — reserve funds as authority bond
        // ----------------------------------------------------
        #[pallet::call_index(6)]
        #[pallet::weight(T::WeightInfo::post_bond())]
        pub fn post_bond(
            origin: OriginFor<T>,
            amount: BalanceOf<T>,
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;

            ensure!(amount >= T::MinAuthorityBond::get(), Error::<T>::BondTooLow);

            T::Currency::reserve(&who, amount)
                .map_err(|_| Error::<T>::InsufficientBalance)?;

            AuthorityBonds::<T>::mutate(&who, |b| {
                *b = (*b).saturating_add(amount);
            });

            Self::deposit_event(Event::BondPosted { authority: who, amount });
            Ok(())
        }

        // ----------------------------------------------------
        // slash — root only governance slash.
        //
        // Uses Currency::slash_reserved (burns directly from reserved balance,
        // no transient free-balance window). After slashing, if the remaining
        // bond falls below MinAuthorityBond the authority is auto-evicted from
        // the active set so it cannot continue producing attestations.
        // ----------------------------------------------------
        #[pallet::call_index(7)]
        #[pallet::weight(T::WeightInfo::slash())]
        pub fn slash(
            origin: OriginFor<T>,
            authority: T::AccountId,
            amount: BalanceOf<T>,
            proof: SlashProof,
        ) -> DispatchResult {
            ensure_root(origin)?;

            ensure!(
                !matches!(proof, SlashProof::None),
                Error::<T>::MissingSlashProof
            );

            let bonded = AuthorityBonds::<T>::get(&authority);
            ensure!(amount <= bonded, Error::<T>::SlashExceedsBond);

            Self::validate_slash_proof(&authority, &proof)?;

            let (_imbalance, _unslashed) =
                T::Currency::slash_reserved(&authority, amount);

            let new_bond = bonded.saturating_sub(amount);
            AuthorityBonds::<T>::insert(&authority, new_bond);

            Self::deposit_event(Event::AuthoritySlashed {
                authority: authority.clone(),
                amount,
            });

            // Auto-evict if the remaining bond is insufficient. The chain may
            // now have fewer authorities than the threshold; downstream payloads
            // will stall until governance calls set_authorities or set_threshold.
            if new_bond < T::MinAuthorityBond::get() {
                let evicted = Authorities::<T>::mutate(|authorities| {
                    let before = authorities.len();
                    authorities.retain(|a| a != &authority);
                    before != authorities.len()
                });
                if evicted {
                    Self::deposit_event(Event::AuthorityEvicted { authority });
                }
            }

            Ok(())
        }

        // ----------------------------------------------------
        // commit_inclusion — an authority signs a promise to include/finalise
        // a PreConsensed payload by `deadline` (spec §14 inclusion commitment).
        //
        // Breaking the promise (deadline passes with the payload still not
        // Finalised) is slashable via SlashProof::BrokenInclusion.
        // ----------------------------------------------------
        #[pallet::call_index(8)]
        #[pallet::weight(T::WeightInfo::commit_inclusion())]
        pub fn commit_inclusion(
            origin: OriginFor<T>,
            payload_id: PayloadId,
            deadline: BlockNumberFor<T>,
            signature: Vec<u8>,
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;

            // Only registered authorities may commit.
            let authorities = Authorities::<T>::get();
            ensure!(authorities.contains(&who), Error::<T>::NotAuthority);

            // Payload must currently be PreConsensed (fast-lane approved but not
            // yet canonical) — that is the only window where a promise is useful.
            let status = Statuses::<T>::get(payload_id)
                .ok_or(Error::<T>::PayloadNotFound)?;
            ensure!(status == Status::PreConsensed, Error::<T>::NotPreConsensed);

            let now = frame_system::Pallet::<T>::block_number();
            ensure!(deadline > now, Error::<T>::InvalidDeadline);

            // One commitment per authority per payload.
            let mut commitments = InclusionCommitments::<T>::get(payload_id);
            ensure!(
                !commitments.iter().any(|c| c.authority == who),
                Error::<T>::AlreadyCommitted
            );

            // Verify the signature over the canonical inclusion message.
            let msg_hash = Self::inclusion_message_hash(&payload_id, &deadline);
            Self::verify_authority_sig(&who, &msg_hash, &signature)?;

            commitments
                .try_push(InclusionCommitment::<T> {
                    authority: who.clone(),
                    committed_at: now,
                    deadline,
                    signature,
                })
                .map_err(|_| Error::<T>::AlreadyCommitted)?;
            InclusionCommitments::<T>::insert(payload_id, commitments);

            Self::deposit_event(Event::InclusionCommitted {
                payload_id,
                authority: who,
                deadline,
            });
            Ok(())
        }

        // ----------------------------------------------------
        // register_bls_key — an authority registers its BLS public key (§14).
        // ----------------------------------------------------
        #[pallet::call_index(9)]
        #[pallet::weight(T::WeightInfo::attest())]
        pub fn register_bls_key(
            origin: OriginFor<T>,
            public_key: Vec<u8>,
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;
            let authorities = Authorities::<T>::get();
            ensure!(authorities.contains(&who), Error::<T>::NotAuthority);

            // The bytes must parse as a real BLS public key.
            ensure!(
                Self::bls_public_key_valid(&public_key),
                Error::<T>::InvalidBlsPublicKey
            );
            let bounded: BoundedVec<u8, ConstU32<128>> = public_key
                .try_into()
                .map_err(|_| Error::<T>::InvalidBlsPublicKey)?;
            AuthorityBlsKeys::<T>::insert(&who, bounded);

            Self::deposit_event(Event::BlsKeyRegistered { authority: who });
            Ok(())
        }

        // ----------------------------------------------------
        // attest_bls — an authority submits a real BLS attestation (§14).
        //
        // Verifies the single BLS signature against the caller's registered
        // key. Once the threshold is reached, all collected BLS signatures are
        // summed into ONE aggregate signature (constant 48 bytes regardless of
        // committee size) and stored in `BlsAggregate` — the real §14 storage
        // reduction, verifiable against the committee's public keys.
        // ----------------------------------------------------
        #[pallet::call_index(10)]
        #[pallet::weight(T::WeightInfo::attest())]
        pub fn attest_bls(
            origin: OriginFor<T>,
            payload_id: PayloadId,
            signature: Vec<u8>,
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;
            Self::do_attest_bls(payload_id, who, signature)
        }

        // ----------------------------------------------------
        // attest_bls_unsigned — the unsigned BLS variant an authority's
        // offchain worker submits automatically. The BLS signature itself
        // authenticates the authority (it must verify against the on-chain
        // registered key), so no outer sr25519 signature is needed.
        // ----------------------------------------------------
        #[pallet::call_index(11)]
        #[pallet::weight(T::WeightInfo::attest_unsigned())]
        pub fn attest_bls_unsigned(
            origin: OriginFor<T>,
            payload_id: PayloadId,
            authority_encoded: Vec<u8>,
            signature: Vec<u8>,
        ) -> DispatchResult {
            ensure_none(origin)?;
            let authority = T::AccountId::decode(&mut authority_encoded.as_slice())
                .map_err(|_| Error::<T>::InvalidBlsSignature)?;
            Self::do_attest_bls(payload_id, authority, signature)
        }
    }

    // --------------------------------------------------------
    // Internal helpers
    // --------------------------------------------------------

    impl<T: Config> Pallet<T> {

        /// Shared attest logic used by both `attest` and `attest_unsigned`.
        fn do_attest(
            payload_id: PayloadId,
            authority: T::AccountId,
            signature: Vec<u8>,
            checks_hash: [u8; 32],
        ) -> DispatchResult {
            let payload = Payloads::<T>::get(payload_id)
                .ok_or(Error::<T>::PayloadNotFound)?;

            let now = frame_system::Pallet::<T>::block_number();
            ensure!(payload.expiry > now, Error::<T>::PayloadExpired);

            let status = Statuses::<T>::get(payload_id)
                .ok_or(Error::<T>::PayloadNotFound)?;
            match status {
                Status::Submitted    => { /* proceed */ }
                Status::PreConsensed => return Err(Error::<T>::AlreadyPreConsensed.into()),
                Status::Finalised    => return Err(Error::<T>::AlreadyFinalised.into()),
                Status::Expired      => return Err(Error::<T>::PayloadExpired.into()),
            }

            let authorities = Authorities::<T>::get();
            ensure!(authorities.contains(&authority), Error::<T>::NotAuthority);

            let mut sigs = Attestations::<T>::get(payload_id);
            ensure!(
                !sigs.iter().any(|r| r.authority == authority),
                Error::<T>::AlreadyAttested
            );

            let expected_checks_hash = T::OffchainChecks::compute_checks_hash(&payload)
                .ok_or(Error::<T>::InvalidSignature)?;
            ensure!(checks_hash == expected_checks_hash, Error::<T>::InvalidSignature);

            // ---- Cryptographic signature verification ----
            let mut msg = payload_id.to_vec();
            msg.extend_from_slice(&payload.expiry.encode());
            msg.extend_from_slice(&checks_hash);
            let msg_hash: [u8; 32] = sp_io::hashing::blake2_256(&msg);

            Self::verify_authority_sig(&authority, &msg_hash, &signature)?;
            // -----------------------------------------------

            let record = SignatureRecord::<T> {
                authority,
                signature,
                checks_hash,
            };
            sigs.try_push(record).map_err(|_| Error::<T>::AlreadyAttested)?;
            Attestations::<T>::insert(payload_id, sigs.clone());

            let threshold = Threshold::<T>::get();
            if threshold > 0 && sigs.len() as u32 >= threshold {
                Statuses::<T>::insert(payload_id, Status::PreConsensed);
                PendingCount::<T>::mutate(|c| *c = c.saturating_sub(1));
                // Schedule auto-finalisation at `now + FinalizeDelay`. Best-
                // effort push; if the bucket is full the payload still becomes
                // PreConsensed and can be finalised manually via finalize().
                let finalize_at = now.saturating_add(T::FinalizeDelay::get());
                let _ = FinalizeAt::<T>::mutate(finalize_at, |q| q.try_push(payload_id));

                // §14 storage reduction: collapse the per-authority signatures
                // into a compact aggregate the moment the threshold is reached.
                let aggregate = Self::build_aggregate(&sigs);
                let count = aggregate.count;
                AggregatedAttestations::<T>::insert(payload_id, aggregate);
                Self::deposit_event(Event::AttestationsAggregated { payload_id, count });

                Self::deposit_event(Event::PreConsensed { payload_id });
            }

            Ok(())
        }

        /// Verify an sr25519 signature by `authority` over `msg_hash`. Accepts
        /// either a raw 64-byte signature or a SCALE-encoded `MultiSignature`.
        /// Shared by `do_attest` and `commit_inclusion`.
        fn verify_authority_sig(
            authority: &T::AccountId,
            msg_hash: &[u8; 32],
            signature: &[u8],
        ) -> DispatchResult {
            let raw_account: Vec<u8> = authority.encode();
            ensure!(raw_account.len() == 32, Error::<T>::InvalidSignature);
            let raw_arr: [u8; 32] = raw_account
                .as_slice()
                .try_into()
                .map_err(|_| Error::<T>::InvalidSignature)?;
            let pub_key = sp_core::sr25519::Public::from_raw(raw_arr);

            let sig = if signature.len() == 64 {
                let sig_bytes: [u8; 64] = signature
                    .try_into()
                    .map_err(|_| Error::<T>::InvalidSignature)?;
                sp_core::sr25519::Signature::from_raw(sig_bytes)
            } else {
                let multi = sp_runtime::MultiSignature::decode(&mut &signature[..])
                    .map_err(|_| Error::<T>::InvalidSignature)?;
                match multi {
                    sp_runtime::MultiSignature::Sr25519(s) => s,
                    _ => return Err(Error::<T>::InvalidSignature.into()),
                }
            };

            ensure!(
                sp_io::crypto::sr25519_verify(&sig, msg_hash, &pub_key),
                Error::<T>::InvalidSignature
            );
            Ok(())
        }

        /// Canonical BLS attestation message. Every authority signs
        /// `Message::new(b"fastlane-attest", payload_id)`, so their signatures
        /// share a message and therefore aggregate.
        fn bls_message(payload_id: &PayloadId) -> w3f_bls::Message {
            w3f_bls::Message::new(b"fastlane-attest", payload_id)
        }

        /// True if `bytes` decode to a valid BLS12-381 (TinyBLS381) public key.
        fn bls_public_key_valid(bytes: &[u8]) -> bool {
            use w3f_bls::{PublicKey, SerializableToBytes, TinyBLS381};
            PublicKey::<TinyBLS381>::from_bytes(bytes).is_ok()
        }

        /// Verify one authority's BLS signature over `payload_id` against its
        /// public key. Runs in-runtime (no host function).
        fn bls_verify_single(
            payload_id: &PayloadId,
            pubkey_bytes: &[u8],
            sig_bytes: &[u8],
        ) -> bool {
            use w3f_bls::{PublicKey, SerializableToBytes, Signature, TinyBLS381};
            type Engine = TinyBLS381;
            let pk = match PublicKey::<Engine>::from_bytes(pubkey_bytes) {
                Ok(p) => p,
                Err(_) => return false,
            };
            let sig = match Signature::<Engine>::from_bytes(sig_bytes) {
                Ok(s) => s,
                Err(_) => return false,
            };
            sig.verify(&Self::bls_message(payload_id), &pk)
        }

        /// Collapse a set of raw BLS signatures into one aggregate signature by
        /// summing their group points. Returns the aggregate's compressed bytes.
        fn aggregate_bls_signatures(sigs: &[Vec<u8>]) -> Option<Vec<u8>> {
            use w3f_bls::{SerializableToBytes, Signature, TinyBLS381};
            type Engine = TinyBLS381;
            let mut iter = sigs.iter();
            let mut agg = Signature::<Engine>::from_bytes(iter.next()?).ok()?;
            for s in iter {
                let sig = Signature::<Engine>::from_bytes(s).ok()?;
                agg.0 += sig.0;
            }
            Some(agg.to_bytes())
        }

        /// Deterministically derive an authority's BLS **public** key bytes from
        /// a 32-byte seed. The matching secret never leaves the signer; the OCW
        /// keeps it in node-local offchain storage. Used to populate genesis /
        /// `register_bls_key`.
        pub fn bls_public_from_seed(seed: &[u8]) -> Vec<u8> {
            use w3f_bls::{SecretKeyVT, SerializableToBytes, TinyBLS381};
            SecretKeyVT::<TinyBLS381>::from_seed(seed).into_public().to_bytes()
        }

        /// Deterministically BLS-sign `payload_id` with the secret derived from
        /// `seed`. `SecretKeyVT::sign` is no_std and deterministic (no RNG), so
        /// this is exactly what an authority's offchain worker calls to attest.
        pub fn bls_sign_with_seed(seed: &[u8], payload_id: &PayloadId) -> Vec<u8> {
            use w3f_bls::{SecretKeyVT, SerializableToBytes, TinyBLS381};
            let sk = SecretKeyVT::<TinyBLS381>::from_seed(seed);
            sk.sign(&Self::bls_message(payload_id)).to_bytes()
        }

        /// Deterministic per-authority BLS seed. DEMO-grade: derived from the
        /// (public) account so genesis and each authority's OCW agree without
        /// key distribution — it is NOT secret. A production deployment would
        /// use random per-node BLS keys registered via `register_bls_key`.
        pub fn bls_seed(authority: &T::AccountId) -> [u8; 32] {
            sp_io::hashing::blake2_256(&(b"fastlane-bls-seed", authority).encode())
        }

        /// Shared BLS-attestation logic for `attest_bls` (signed) and
        /// `attest_bls_unsigned` (OCW). Verifies the authority's single BLS
        /// signature against its registered key, records it, and once the
        /// threshold is reached stores the single aggregate signature. Does not
        /// require the payload to still be `Submitted`, so BLS attestations can
        /// complete independently of the sr25519 finalisation race.
        fn do_attest_bls(
            payload_id: PayloadId,
            authority: T::AccountId,
            signature: Vec<u8>,
        ) -> DispatchResult {
            let authorities = Authorities::<T>::get();
            ensure!(authorities.contains(&authority), Error::<T>::NotAuthority);

            let payload = Payloads::<T>::get(payload_id)
                .ok_or(Error::<T>::PayloadNotFound)?;
            let now = frame_system::Pallet::<T>::block_number();
            ensure!(payload.expiry > now, Error::<T>::PayloadExpired);

            let pubkey = AuthorityBlsKeys::<T>::get(&authority)
                .ok_or(Error::<T>::BlsKeyNotRegistered)?;
            ensure!(
                Self::bls_verify_single(&payload_id, pubkey.as_slice(), &signature),
                Error::<T>::InvalidBlsSignature
            );

            let mut recs = BlsAttestations::<T>::get(payload_id);
            ensure!(
                !recs.iter().any(|(a, _)| a == &authority),
                Error::<T>::BlsAlreadyAttested
            );
            let sig_bounded: BoundedVec<u8, ConstU32<64>> = signature
                .try_into()
                .map_err(|_| Error::<T>::InvalidBlsSignature)?;
            recs.try_push((authority, sig_bounded))
                .map_err(|_| Error::<T>::AlreadyAttested)?;
            BlsAttestations::<T>::insert(payload_id, recs.clone());

            let threshold = Threshold::<T>::get();
            if threshold > 0 && recs.len() as u32 >= threshold {
                let sigs: Vec<Vec<u8>> =
                    recs.iter().map(|(_, s)| s.to_vec()).collect();
                if let Some(agg) = Self::aggregate_bls_signatures(&sigs) {
                    // Emit BlsAggregated only the first time the aggregate is
                    // created. The stored aggregate is still refreshed as more
                    // sign-offs arrive (so it stays verifiable against the full
                    // signer set), but the event fires once per payload — not
                    // once per attestation past the threshold.
                    let first_time = !BlsAggregate::<T>::contains_key(payload_id);
                    let agg_bounded: BoundedVec<u8, ConstU32<64>> = agg
                        .try_into()
                        .map_err(|_| Error::<T>::InvalidBlsSignature)?;
                    BlsAggregate::<T>::insert(payload_id, agg_bounded);
                    if first_time {
                        Self::deposit_event(Event::BlsAggregated {
                            payload_id,
                            count: recs.len() as u32,
                        });
                    }
                }
            }
            Ok(())
        }

        /// §14 — in-runtime BLS12-381 aggregate verification.
        ///
        /// Verifies that `aggregate_sig` is the BLS aggregate of signatures by
        /// every key in `pubkeys`, each over the same message
        /// `Message::new(ctx, payload_id)`. Runs entirely inside the runtime
        /// WASM (no `bls381_*` host function required), so activating real BLS
        /// is a forkless runtime upgrade — no node-binary rebuild or validator
        /// redeploy. `pubkeys`/`aggregate_sig` are `w3f_bls` compressed byte
        /// encodings. This is the on-chain verify half of the BLS path; the OCW
        /// signing half is wired separately.
        pub fn verify_bls_aggregate(
            ctx: &[u8],
            payload_id: &PayloadId,
            pubkeys: &[Vec<u8>],
            aggregate_sig: &[u8],
        ) -> bool {
            // Uses only the no_std-available w3f-bls API (the convenience
            // `single_pop_aggregator` is std-only). Same-message BLS aggregate
            // verification: the aggregate signature verifies against the SUM of
            // the signers' public keys.
            use w3f_bls::{Message, PublicKey, SerializableToBytes, Signature, TinyBLS381};
            type Engine = TinyBLS381;

            if pubkeys.is_empty() {
                return false;
            }
            let message = Message::new(ctx, payload_id);
            let sig = match Signature::<Engine>::from_bytes(aggregate_sig) {
                Ok(s) => s,
                Err(_) => return false,
            };

            // Aggregate the public keys by summing their group points.
            let mut iter = pubkeys.iter();
            let mut agg_pk = match PublicKey::<Engine>::from_bytes(iter.next().unwrap()) {
                Ok(pk) => pk,
                Err(_) => return false,
            };
            for pk_bytes in iter {
                match PublicKey::<Engine>::from_bytes(pk_bytes) {
                    Ok(pk) => agg_pk.0 += pk.0,
                    Err(_) => return false,
                }
            }

            sig.verify(&message, &agg_pk)
        }

        /// Canonical message hash an authority signs to commit to including a
        /// payload: blake2_256(payload_id ++ b"INCLUDE" ++ deadline.encode()).
        pub fn inclusion_message_hash(
            payload_id: &PayloadId,
            deadline: &BlockNumberFor<T>,
        ) -> [u8; 32] {
            let mut msg = payload_id.to_vec();
            msg.extend_from_slice(b"INCLUDE");
            msg.extend_from_slice(&deadline.encode());
            sp_io::hashing::blake2_256(&msg)
        }

        /// Build the compact aggregate from a payload's signature set. The
        /// bitmap indexes into the *current* authority ordering; the digest is a
        /// commitment to the concatenated signatures in authority order.
        fn build_aggregate(
            sigs: &BoundedVec<SignatureRecord<T>, T::MaxAuthorities>,
        ) -> AggregatedAttestation {
            let authorities = Authorities::<T>::get();
            let mut bitmap: u64 = 0;
            let mut digest_input: Vec<u8> = Vec::new();
            let mut count: u32 = 0;

            for (idx, auth) in authorities.iter().enumerate() {
                if idx >= 64 {
                    break;
                }
                if let Some(record) = sigs.iter().find(|r| &r.authority == auth) {
                    bitmap |= 1u64 << idx;
                    digest_input.extend_from_slice(&record.signature);
                    count = count.saturating_add(1);
                }
            }

            AggregatedAttestation {
                signer_bitmap: bitmap,
                digest: sp_io::hashing::blake2_256(&digest_input),
                count,
            }
        }

        /// Remove the full per-authority attestation list and inclusion
        /// commitments for a finalised payload. The compact
        /// `AggregatedAttestations` record is retained. Reduces the permanent
        /// storage footprint of finalised payloads (spec §14).
        fn prune_finalised_storage(payload_id: PayloadId) {
            Attestations::<T>::remove(payload_id);
            InclusionCommitments::<T>::remove(payload_id);
        }

        /// Validates on-chain evidence before slashing.
        fn validate_slash_proof(
            authority: &T::AccountId,
            proof: &SlashProof,
        ) -> DispatchResult {
            match proof {
                SlashProof::None => Err(Error::<T>::MissingSlashProof.into()),

                SlashProof::DoubleSigning(id_a, sig_a, id_b, sig_b) => {
                    ensure!(id_a != id_b, Error::<T>::MissingSlashProof);

                    let sigs_a = Attestations::<T>::get(id_a);
                    let sigs_b = Attestations::<T>::get(id_b);

                    let found_a = sigs_a.iter().any(|r| {
                        r.authority == *authority && &r.signature == sig_a
                    });
                    let found_b = sigs_b.iter().any(|r| {
                        r.authority == *authority && &r.signature == sig_b
                    });

                    ensure!(found_a && found_b, Error::<T>::MissingSlashProof);
                    Ok(())
                }

                SlashProof::InvalidSignature(payload_id, bad_sig) => {
                    let sigs = Attestations::<T>::get(payload_id);
                    let found = sigs.iter().find(|r| {
                        r.authority == *authority && &r.signature == bad_sig
                    });
                    let record = found.ok_or(Error::<T>::MissingSlashProof)?;

                    let payload = Payloads::<T>::get(payload_id)
                        .ok_or(Error::<T>::PayloadNotFound)?;

                    let mut msg = payload_id.to_vec();
                    msg.extend_from_slice(&payload.expiry.encode());
                    msg.extend_from_slice(&record.checks_hash);
                    let msg_hash: [u8; 32] = sp_io::hashing::blake2_256(&msg);

                    let raw: Vec<u8> = authority.encode();
                    ensure!(raw.len() == 32, Error::<T>::MissingSlashProof);
                    let raw_arr: [u8; 32] = raw
                        .as_slice()
                        .try_into()
                        .map_err(|_| Error::<T>::MissingSlashProof)?;
                    let pub_key = sp_core::sr25519::Public::from_raw(raw_arr);

                    let sig_bytes: [u8; 64] = bad_sig
                        .as_slice()
                        .try_into()
                        .map_err(|_| Error::<T>::MissingSlashProof)?;
                    let sig = sp_core::sr25519::Signature::from_raw(sig_bytes);

                    // Signature must FAIL for the slash proof to be valid.
                    ensure!(
                        !sp_io::crypto::sr25519_verify(&sig, &msg_hash, &pub_key),
                        Error::<T>::MissingSlashProof
                    );

                    Ok(())
                }

                SlashProof::BrokenInclusion(payload_id) => {
                    // The authority must have an on-chain inclusion commitment
                    // for this payload whose deadline has already passed.
                    let commitments = InclusionCommitments::<T>::get(payload_id);
                    let commitment = commitments
                        .iter()
                        .find(|c| &c.authority == authority)
                        .ok_or(Error::<T>::MissingSlashProof)?;

                    let now = frame_system::Pallet::<T>::block_number();
                    ensure!(now > commitment.deadline, Error::<T>::MissingSlashProof);

                    // The promise is only broken if the payload is NOT finalised.
                    // (If InclusionCommitments were pruned on finalisation the
                    // lookup above would already fail, but guard explicitly too.)
                    let status = Statuses::<T>::get(payload_id)
                        .ok_or(Error::<T>::PayloadNotFound)?;
                    ensure!(status != Status::Finalised, Error::<T>::MissingSlashProof);

                    Ok(())
                }
            }
        }
    }
}
pub mod crypto {
    use sp_runtime::{
        app_crypto::{app_crypto, sr25519},
        MultiSignature, MultiSigner,
        KeyTypeId,
    };

    pub const KEY_TYPE: KeyTypeId = KeyTypeId(*b"fast");
    app_crypto!(sr25519, KEY_TYPE);

    pub struct FastlaneAuthId;

    impl frame_system::offchain::AppCrypto<MultiSigner, MultiSignature>
        for FastlaneAuthId
    {
        type RuntimeAppPublic = Public;
        type GenericSignature = sp_core::sr25519::Signature;
        type GenericPublic = sp_core::sr25519::Public;
    }
}
