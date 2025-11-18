use sp_runtime::transaction_validity::{TransactionLongevity, TransactionPriority};
use frame_support::parameter_types;

use super::{
    Runtime, RuntimeEvent,
};

parameter_types! {
	// This currently must be set to DEFAULT_STORAGE_PERIOD.\	
    pub const StoragePeriod: crate::BlockNumber = sp_transaction_storage_proof::DEFAULT_STORAGE_PERIOD;
    pub const AuthorizationPeriod: crate::BlockNumber = 7 * crate::DAYS;
    // Priorities and longevities used by the transaction storage pallet extrinsics.
	pub const SudoPriority: TransactionPriority = TransactionPriority::MAX;
	pub const SetPurgeKeysPriority: TransactionPriority = SudoPriority::get() - 1;
	pub const SetPurgeKeysLongevity: TransactionLongevity = crate::HOURS as TransactionLongevity;
	pub const RemoveExpiredAuthorizationPriority: TransactionPriority = SetPurgeKeysPriority::get() - 1;
	pub const RemoveExpiredAuthorizationLongevity: TransactionLongevity = crate::DAYS as TransactionLongevity;
	pub const StoreRenewPriority: TransactionPriority = RemoveExpiredAuthorizationPriority::get() - 1;
	pub const StoreRenewLongevity: TransactionLongevity = crate::DAYS as TransactionLongevity;
}


impl pallet_transaction_storage::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	type WeightInfo = crate::weights::pallet_transaction_storage::WeightInfo<Runtime>;
	type MaxBlockTransactions = crate::ConstU32<512>;
	/// Max transaction size per block needs to be aligned with `BlockLength`.
	type MaxTransactionSize = crate::ConstU32<{ 8 * 1024 * 1024 }>;
	type StoragePeriod = StoragePeriod;
	type AuthorizationPeriod = AuthorizationPeriod;
	type Authorizer = crate::EnsureRoot<Self::AccountId>;
	type StoreRenewPriority = StoreRenewPriority;
	type StoreRenewLongevity = StoreRenewLongevity;
	type RemoveExpiredAuthorizationPriority = RemoveExpiredAuthorizationPriority;
	type RemoveExpiredAuthorizationLongevity = RemoveExpiredAuthorizationLongevity;
}