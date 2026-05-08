// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

//! Conformance tests for the XCM `authorize_account` integration on Bulletin
//! Chain runtimes.
//!
//! Each helper is a generic function over the runtime and its `XcmConfig`,
//! mirroring the pattern in `parachains-runtimes-test-utils`. The authorizer's
//! `Location` is a parameter so the same suite covers People Chain today and
//! any future authorizer (Fellows, others) without duplication.
//!
//! Runtime crates depend on this from `[dev-dependencies]` and call each
//! function from a thin `#[test]` wrapper that supplies the runtime types,
//! the authorizer location, an externalities builder, and an
//! `advance_block` callback.

use codec::Encode;
use frame_system::RawOrigin;
use pallet_bulletin_transaction_storage::{
	AuthorizationExtent, Call as TxStorageCall, Config as TxStorageConfig,
	Pallet as TxStoragePallet,
};
use parachains_common::AccountId;
use polkadot_sdk_frame::prelude::Get;
use sp_io::TestExternalities;
use sp_keyring::Sr25519Keyring;
use xcm::latest::prelude::*;
use xcm_executor::traits::ConvertLocation;

/// Convenience alias for the runtime's `RuntimeCall`.
pub type RuntimeCallOf<R> = <R as frame_system::Config>::RuntimeCall;

/// Bound bundle every conformance test in this module shares: `R` is a
/// runtime that includes `pallet_bulletin_transaction_storage` and
/// `pallet_utility`, uses `AccountId32`, and has a single `RuntimeCall` enum
/// across the `frame_system` and `pallet_utility` configs (the standard setup
/// produced by `construct_runtime!`). The `RuntimeCall` constraints sit on
/// the supertrait so they propagate to consumers.
pub trait BulletinXcmTestRuntime:
	TxStorageConfig
	+ pallet_utility::Config<RuntimeCall = <Self as frame_system::Config>::RuntimeCall>
	+ frame_system::Config<
		AccountId = AccountId,
		RuntimeCall: From<TxStorageCall<Self>> + From<pallet_utility::Call<Self>> + Encode,
	>
{
}

impl<R> BulletinXcmTestRuntime for R where
	R: TxStorageConfig
		+ pallet_utility::Config<RuntimeCall = <Self as frame_system::Config>::RuntimeCall>
		+ frame_system::Config<
			AccountId = AccountId,
			RuntimeCall: From<TxStorageCall<Self>> + From<pallet_utility::Call<Self>> + Encode,
		>
{
}

// ---------------------------------------------------------------------------
// XCM helpers
// ---------------------------------------------------------------------------

fn xcm_transact<Call: Encode>(call: Call, kind: OriginKind) -> Xcm<Call> {
	Xcm::builder_unsafe()
		.unpaid_execution(Unlimited, None)
		.transact(kind, None, call.encode())
		.build()
}

fn execute_from<Cfg: xcm_executor::Config>(
	origin: Location,
	message: Xcm<<Cfg as xcm_executor::Config>::RuntimeCall>,
) -> Outcome {
	let mut id = [0u8; 32];
	xcm_executor::XcmExecutor::<Cfg>::prepare_and_execute(
		origin,
		message,
		&mut id,
		Weight::MAX,
		Weight::MAX,
	)
}

fn authorize_call<R: BulletinXcmTestRuntime>(
	who: AccountId,
	transactions: u32,
	bytes: u64,
) -> RuntimeCallOf<R> {
	TxStorageCall::<R>::authorize_account { who, transactions, bytes }.into()
}

fn refresh_call<R: BulletinXcmTestRuntime>(who: AccountId) -> RuntimeCallOf<R> {
	TxStorageCall::<R>::refresh_account_authorization { who }.into()
}

fn store_call<R: BulletinXcmTestRuntime>(data: Vec<u8>) -> RuntimeCallOf<R> {
	TxStorageCall::<R>::store { data }.into()
}

fn batch_call<R: BulletinXcmTestRuntime>(calls: Vec<RuntimeCallOf<R>>) -> RuntimeCallOf<R> {
	pallet_utility::Call::<R>::batch { calls }.into()
}

/// Send an XCM `Transact { authorize_account }` from the given authorizer
/// location to the runtime under test. Public so runtime crates can compose it
/// into runtime-specific flows (e.g. authorize then submit a signed extrinsic).
pub fn xcm_authorize<R, Cfg>(
	authorizer: Location,
	who: AccountId,
	transactions: u32,
	bytes: u64,
) -> Outcome
where
	R: BulletinXcmTestRuntime,
	Cfg: xcm_executor::Config<RuntimeCall = RuntimeCallOf<R>>,
{
	execute_from::<Cfg>(
		authorizer,
		xcm_transact(authorize_call::<R>(who, transactions, bytes), OriginKind::Xcm),
	)
}

/// Send an XCM `Transact { refresh_account_authorization }` from the given
/// authorizer location.
pub fn xcm_refresh<R, Cfg>(authorizer: Location, who: AccountId) -> Outcome
where
	R: BulletinXcmTestRuntime,
	Cfg: xcm_executor::Config<RuntimeCall = RuntimeCallOf<R>>,
{
	execute_from::<Cfg>(authorizer, xcm_transact(refresh_call::<R>(who), OriginKind::Xcm))
}

// ---------------------------------------------------------------------------
// State helpers
// ---------------------------------------------------------------------------

fn extent_of<R: BulletinXcmTestRuntime>(who: &AccountId) -> AuthorizationExtent {
	TxStoragePallet::<R>::account_authorization_extent(who.clone())
}

fn empty() -> AuthorizationExtent {
	AuthorizationExtent::default()
}

fn extent(
	bytes: u64,
	bytes_allowance: u64,
	transactions: u32,
	transactions_allowance: u32,
) -> AuthorizationExtent {
	AuthorizationExtent {
		bytes,
		bytes_permanent: 0,
		bytes_allowance,
		transactions,
		transactions_allowance,
	}
}

fn authorization_period<R: BulletinXcmTestRuntime>(
) -> frame_system::pallet_prelude::BlockNumberFor<R> {
	<R as TxStorageConfig>::AuthorizationPeriod::get()
}

/// Consume `bytes` of `store` quota for `who` directly via the pallet,
/// without going through the runtime's signed-extrinsic stack. Used by
/// scenarios that need the consumed counters to advance before re-authorizing.
fn consume_store_via_pallet<R: BulletinXcmTestRuntime>(who: &AccountId, bytes: usize) {
	let call = TxStorageCall::<R>::store { data: vec![0u8; bytes] };
	TxStoragePallet::<R>::pre_dispatch_signed(who, &call)
		.expect("pre_dispatch_signed must consume authorization in tests");
}

/// Set the system block number directly. Used to jump past
/// `AuthorizationPeriod` without iterating block hooks (which would be
/// hundreds of thousands of iterations on real-runtime parameters).
fn set_block_number<R: BulletinXcmTestRuntime>(n: frame_system::pallet_prelude::BlockNumberFor<R>) {
	frame_system::Pallet::<R>::set_block_number(n);
}

fn block_number<R: BulletinXcmTestRuntime>() -> frame_system::pallet_prelude::BlockNumberFor<R> {
	frame_system::Pallet::<R>::block_number()
}

// ---------------------------------------------------------------------------
// Conformance tests
// ---------------------------------------------------------------------------

/// Happy path. The authorizer grants caps to a single account from a fresh
/// state; the stored extent matches the granted caps and the entry is active.
pub fn xcm_authorize_happy_path<R, Cfg>(
	authorizer: Location,
	new_ext: impl FnOnce() -> TestExternalities,
	advance_block: impl Fn(),
) where
	R: BulletinXcmTestRuntime,
	Cfg: xcm_executor::Config<RuntimeCall = RuntimeCallOf<R>>,
{
	let mut ext = new_ext();
	ext.execute_with(|| {
		advance_block();
		let who = Sr25519Keyring::Alice.to_account_id();

		assert!(xcm_authorize::<R, Cfg>(authorizer, who.clone(), 10, 1_000_000)
			.ensure_complete()
			.is_ok());

		assert_eq!(extent_of::<R>(&who), extent(0, 1_000_000, 0, 10));
		assert!(TxStoragePallet::<R>::account_has_active_authorization(&who));
	});
}

/// Authorizations are additive within an unexpired window. Each grant adds to
/// the existing allowance and does NOT push expiry forward. Consumed
/// counters are preserved.
pub fn xcm_authorize_is_additive_within_window<R, Cfg>(
	authorizer: Location,
	new_ext: impl FnOnce() -> TestExternalities,
	advance_block: impl Fn(),
) where
	R: BulletinXcmTestRuntime,
	Cfg: xcm_executor::Config<RuntimeCall = RuntimeCallOf<R>>,
{
	let mut ext = new_ext();
	ext.execute_with(|| {
		advance_block();
		let who = Sr25519Keyring::Alice.to_account_id();

		assert!(xcm_authorize::<R, Cfg>(authorizer.clone(), who.clone(), 5, 1_000)
			.ensure_complete()
			.is_ok());
		assert_eq!(extent_of::<R>(&who), extent(0, 1_000, 0, 5));

		consume_store_via_pallet::<R>(&who, 200);
		assert_eq!(extent_of::<R>(&who), extent(200, 1_000, 1, 5));

		assert!(xcm_authorize::<R, Cfg>(authorizer.clone(), who.clone(), 3, 500)
			.ensure_complete()
			.is_ok());
		assert_eq!(extent_of::<R>(&who), extent(200, 1_500, 1, 8));

		assert!(xcm_authorize::<R, Cfg>(authorizer, who.clone(), 2, 250)
			.ensure_complete()
			.is_ok());
		assert_eq!(extent_of::<R>(&who), extent(200, 1_750, 1, 10));

		// Expiry must not have been pushed forward by the additive grants.
		set_block_number::<R>(block_number::<R>() + authorization_period::<R>());
		assert_eq!(extent_of::<R>(&who), empty());
	});
}

/// Replace after expiry. On an expired-but-present entry, a new grant resets
/// all consumed counters and replaces (not adds) the allowances.
pub fn xcm_authorize_replaces_after_expiry<R, Cfg>(
	authorizer: Location,
	new_ext: impl FnOnce() -> TestExternalities,
	advance_block: impl Fn(),
) where
	R: BulletinXcmTestRuntime,
	Cfg: xcm_executor::Config<RuntimeCall = RuntimeCallOf<R>>,
	frame_system::pallet_prelude::BlockNumberFor<R>: From<u32>,
{
	let mut ext = new_ext();
	ext.execute_with(|| {
		advance_block();
		let who = Sr25519Keyring::Alice.to_account_id();

		assert!(xcm_authorize::<R, Cfg>(authorizer.clone(), who.clone(), 5, 1_000)
			.ensure_complete()
			.is_ok());

		consume_store_via_pallet::<R>(&who, 400);
		assert_eq!(extent_of::<R>(&who), extent(400, 1_000, 1, 5));

		set_block_number::<R>(block_number::<R>() + authorization_period::<R>() + 1u32.into());
		assert_eq!(extent_of::<R>(&who), empty());

		assert!(xcm_authorize::<R, Cfg>(authorizer, who.clone(), 1, 100)
			.ensure_complete()
			.is_ok());
		assert_eq!(extent_of::<R>(&who), extent(0, 100, 0, 1));
	});
}

/// Independent account scopes. Two accounts authorized separately. Removing
/// one does not affect the other.
pub fn xcm_account_scopes_are_independent<R, Cfg>(
	authorizer: Location,
	new_ext: impl FnOnce() -> TestExternalities,
	advance_block: impl Fn(),
) where
	R: BulletinXcmTestRuntime,
	Cfg: xcm_executor::Config<RuntimeCall = RuntimeCallOf<R>>,
	frame_system::pallet_prelude::BlockNumberFor<R>: From<u32>,
{
	let mut ext = new_ext();
	ext.execute_with(|| {
		advance_block();
		let alice = Sr25519Keyring::Alice.to_account_id();
		let bob = Sr25519Keyring::Bob.to_account_id();

		assert!(xcm_authorize::<R, Cfg>(authorizer.clone(), alice.clone(), 5, 1_000)
			.ensure_complete()
			.is_ok());
		assert!(xcm_authorize::<R, Cfg>(authorizer.clone(), bob.clone(), 10, 2_000)
			.ensure_complete()
			.is_ok());
		assert_eq!(extent_of::<R>(&alice), extent(0, 1_000, 0, 5));
		assert_eq!(extent_of::<R>(&bob), extent(0, 2_000, 0, 10));

		set_block_number::<R>(block_number::<R>() + authorization_period::<R>() + 1u32.into());
		// Permissionless `remove_expired_account_authorization` accepts any
		// origin; pass `RawOrigin::None`.
		TxStoragePallet::<R>::remove_expired_account_authorization(
			RawOrigin::None.into(),
			alice.clone(),
		)
		.expect("Alice's authorization is past its expiry and should be removable");
		assert_eq!(extent_of::<R>(&alice), empty());

		assert!(xcm_authorize::<R, Cfg>(authorizer, bob.clone(), 1, 50)
			.ensure_complete()
			.is_ok());
		assert_eq!(extent_of::<R>(&bob), extent(0, 50, 0, 1));
	});
}

/// Relay-chain origin cannot authorize. The dispatch resolves to
/// `pallet_xcm::Origin::Xcm(parent)` which does not match the runtime's
/// authorizer filter; the inner call fails with `BadOrigin`. XCM's `Outcome`
/// stays `Complete` because the failure is at dispatch level, not at the
/// XCM-instruction level. The signal is the absence of any storage mutation.
pub fn relay_chain_origin_cannot_authorize<R, Cfg>(
	new_ext: impl FnOnce() -> TestExternalities,
	advance_block: impl Fn(),
) where
	R: BulletinXcmTestRuntime,
	Cfg: xcm_executor::Config<RuntimeCall = RuntimeCallOf<R>>,
{
	let mut ext = new_ext();
	ext.execute_with(|| {
		advance_block();
		let target = Sr25519Keyring::Ferdie.to_account_id();
		let outcome = execute_from::<Cfg>(
			Location::parent(),
			xcm_transact(authorize_call::<R>(target.clone(), 5, 1_000), OriginKind::Xcm),
		);
		assert!(outcome.ensure_complete().is_ok());
		assert_eq!(extent_of::<R>(&target), empty());
	});
}

/// Sibling sending Transact with `OriginKind::SovereignAccount` resolves to a
/// `Signed(authorizer_sovereign)` origin. The sovereign is not an
/// `pallet_xcm::Origin::Xcm`, so the dispatch fails with `BadOrigin`. The
/// derived sovereign account is also asserted not to gain an authorization as
/// a side effect.
pub fn sovereign_origin_kind_cannot_authorize<R, Cfg, LocationToAccount>(
	authorizer: Location,
	new_ext: impl FnOnce() -> TestExternalities,
	advance_block: impl Fn(),
) where
	R: BulletinXcmTestRuntime,
	Cfg: xcm_executor::Config<RuntimeCall = RuntimeCallOf<R>>,
	LocationToAccount: ConvertLocation<AccountId>,
{
	let mut ext = new_ext();
	ext.execute_with(|| {
		advance_block();
		let target = Sr25519Keyring::Ferdie.to_account_id();
		let outcome = execute_from::<Cfg>(
			authorizer.clone(),
			xcm_transact(
				authorize_call::<R>(target.clone(), 5, 1_000),
				OriginKind::SovereignAccount,
			),
		);
		assert!(outcome.ensure_complete().is_ok());
		assert_eq!(extent_of::<R>(&target), empty());

		let sovereign = LocationToAccount::convert_location(&authorizer)
			.expect("sibling sovereign account must derive");
		assert_eq!(extent_of::<R>(&sovereign), empty());
	});
}

/// Random local AccountId32 origin cannot authorize.
pub fn random_local_origin_cannot_authorize<R, Cfg>(
	new_ext: impl FnOnce() -> TestExternalities,
	advance_block: impl Fn(),
) where
	R: BulletinXcmTestRuntime,
	Cfg: xcm_executor::Config<RuntimeCall = RuntimeCallOf<R>>,
{
	let mut ext = new_ext();
	ext.execute_with(|| {
		advance_block();
		let target = Sr25519Keyring::Ferdie.to_account_id();
		let stranger_loc =
			Location::new(0, [Junction::AccountId32 { network: None, id: [0x42u8; 32] }]);
		let outcome = execute_from::<Cfg>(
			stranger_loc,
			xcm_transact(authorize_call::<R>(target.clone(), 5, 1_000), OriginKind::Xcm),
		);
		assert!(outcome.ensure_complete().is_ok());
		assert_eq!(extent_of::<R>(&target), empty());
	});
}

/// `SafeCallFilter` blocks `store` arriving over XCM, even when the caller is
/// otherwise valid and the target has a real allowance.
pub fn xcm_store_is_blocked<R, Cfg>(
	authorizer: Location,
	new_ext: impl FnOnce() -> TestExternalities,
	advance_block: impl Fn(),
) where
	R: BulletinXcmTestRuntime,
	Cfg: xcm_executor::Config<RuntimeCall = RuntimeCallOf<R>>,
{
	let mut ext = new_ext();
	ext.execute_with(|| {
		advance_block();
		let who = Sr25519Keyring::Alice.to_account_id();
		assert!(xcm_authorize::<R, Cfg>(authorizer.clone(), who.clone(), 1, 1_000)
			.ensure_complete()
			.is_ok());

		let outcome = execute_from::<Cfg>(
			authorizer,
			xcm_transact(store_call::<R>(vec![0u8; 100]), OriginKind::Xcm),
		);
		assert!(
			outcome.clone().ensure_complete().is_err(),
			"SafeCallFilter must block sibling-XCM `store`, got: {outcome:?}"
		);
		assert_eq!(extent_of::<R>(&who), extent(0, 1_000, 0, 1));
	});
}

/// `Utility::batch([authorize_account, store])` is rejected as a whole;
/// neither half executes.
pub fn xcm_batch_with_store_is_entirely_blocked<R, Cfg>(
	authorizer: Location,
	new_ext: impl FnOnce() -> TestExternalities,
	advance_block: impl Fn(),
) where
	R: BulletinXcmTestRuntime,
	Cfg: xcm_executor::Config<RuntimeCall = RuntimeCallOf<R>>,
{
	let mut ext = new_ext();
	ext.execute_with(|| {
		advance_block();
		let target = Sr25519Keyring::Bob.to_account_id();
		let store_target = Sr25519Keyring::Alice.to_account_id();

		let inner =
			vec![authorize_call::<R>(target.clone(), 5, 1_000), store_call::<R>(vec![0u8; 50])];
		let outcome =
			execute_from::<Cfg>(authorizer, xcm_transact(batch_call::<R>(inner), OriginKind::Xcm));

		assert!(
			outcome.clone().ensure_complete().is_err(),
			"SafeCallFilter must reject the whole batch when any inner call is filtered, got: {outcome:?}",
		);
		assert_eq!(extent_of::<R>(&target), empty());
		assert_eq!(extent_of::<R>(&store_target), empty());
	});
}

/// `Utility::batch([authorize_account, authorize_account])` is allowed and
/// both extents land. Boundary case proving the filter does not over-block
/// legitimate XCM-driven authorization batches.
pub fn xcm_batch_of_only_authorize_calls_succeeds<R, Cfg>(
	authorizer: Location,
	new_ext: impl FnOnce() -> TestExternalities,
	advance_block: impl Fn(),
) where
	R: BulletinXcmTestRuntime,
	Cfg: xcm_executor::Config<RuntimeCall = RuntimeCallOf<R>>,
{
	let mut ext = new_ext();
	ext.execute_with(|| {
		advance_block();
		let alice = Sr25519Keyring::Alice.to_account_id();
		let bob = Sr25519Keyring::Bob.to_account_id();

		let inner = vec![
			authorize_call::<R>(alice.clone(), 5, 1_000),
			authorize_call::<R>(bob.clone(), 10, 2_000),
		];
		let outcome =
			execute_from::<Cfg>(authorizer, xcm_transact(batch_call::<R>(inner), OriginKind::Xcm));

		assert!(
			outcome.clone().ensure_complete().is_ok(),
			"sibling XCM batch of authorize_account calls must complete, got: {outcome:?}"
		);
		assert_eq!(extent_of::<R>(&alice), extent(0, 1_000, 0, 5));
		assert_eq!(extent_of::<R>(&bob), extent(0, 2_000, 0, 10));
	});
}

/// `authorize_account(_, bytes: 0)` is rejected by the pallet at dispatch
/// level.
pub fn xcm_authorize_with_zero_bytes_fails<R, Cfg>(
	authorizer: Location,
	new_ext: impl FnOnce() -> TestExternalities,
	advance_block: impl Fn(),
) where
	R: BulletinXcmTestRuntime,
	Cfg: xcm_executor::Config<RuntimeCall = RuntimeCallOf<R>>,
{
	let mut ext = new_ext();
	ext.execute_with(|| {
		advance_block();
		let who = Sr25519Keyring::Alice.to_account_id();
		assert!(xcm_authorize::<R, Cfg>(authorizer, who.clone(), 1, 0).ensure_complete().is_ok());
		assert_eq!(extent_of::<R>(&who), empty());
		assert!(!TxStoragePallet::<R>::account_has_active_authorization(&who));
	});
}

/// `refresh_account_authorization` only extends expiry. Allowances and
/// consumed counters are unchanged.
pub fn xcm_refresh_extends_only_expiration<R, Cfg>(
	authorizer: Location,
	new_ext: impl FnOnce() -> TestExternalities,
	advance_block: impl Fn(),
) where
	R: BulletinXcmTestRuntime,
	Cfg: xcm_executor::Config<RuntimeCall = RuntimeCallOf<R>>,
	frame_system::pallet_prelude::BlockNumberFor<R>: From<u32>,
{
	let mut ext = new_ext();
	ext.execute_with(|| {
		advance_block();
		let who = Sr25519Keyring::Alice.to_account_id();

		assert!(xcm_authorize::<R, Cfg>(authorizer.clone(), who.clone(), 5, 1_000)
			.ensure_complete()
			.is_ok());
		assert_eq!(extent_of::<R>(&who), extent(0, 1_000, 0, 5));

		let now = block_number::<R>();
		let half = authorization_period::<R>() / 2u32.into();
		set_block_number::<R>(now + half);

		assert!(xcm_refresh::<R, Cfg>(authorizer, who.clone()).ensure_complete().is_ok());
		assert_eq!(extent_of::<R>(&who), extent(0, 1_000, 0, 5));

		// Past the original expiry: still active because refresh extended it.
		set_block_number::<R>(now + authorization_period::<R>());
		assert!(TxStoragePallet::<R>::account_has_active_authorization(&who));
	});
}

/// `refresh_account_authorization` without a prior authorize fails; no entry
/// is created.
pub fn xcm_refresh_without_prior_authorize_fails<R, Cfg>(
	authorizer: Location,
	new_ext: impl FnOnce() -> TestExternalities,
	advance_block: impl Fn(),
) where
	R: BulletinXcmTestRuntime,
	Cfg: xcm_executor::Config<RuntimeCall = RuntimeCallOf<R>>,
{
	let mut ext = new_ext();
	ext.execute_with(|| {
		advance_block();
		let who = Sr25519Keyring::Alice.to_account_id();
		assert!(xcm_refresh::<R, Cfg>(authorizer, who.clone()).ensure_complete().is_ok());
		assert_eq!(extent_of::<R>(&who), empty());
		assert!(!TxStoragePallet::<R>::account_has_active_authorization(&who));
	});
}
