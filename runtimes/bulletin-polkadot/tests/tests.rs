use bp_header_chain::{justification::GrandpaJustification, HeaderChain, InitializationData};
use bp_messages::{
	ChainWithMessages, DeliveredMessages, InboundLaneData, LaneState, OutboundLaneData,
	UnrewardedRelayer,
};
use bp_polkadot_core::parachains::{ParaHead, ParaHeadsProof};
use bp_runtime::{
	record_all_trie_keys, BasicOperatingMode, HeaderIdProvider, RawStorageProof,
	UnverifiedStorageProofParams,
};
use bulletin_polkadot_runtime as runtime;
use bulletin_polkadot_runtime::{
	bridge_config::{
		WithPeoplePolkadotMessagesInstance, WithPolkadotBridgeParachainsInstance, XCM_LANE,
	},
	AccountId, BridgePolkadotGrandpa, BridgePolkadotMessages,
};
use frame_support::{assert_ok, dispatch::GetDispatchInfo, traits::Get};
use pallet_bridge_messages::{
	messages_generation::{encode_all_messages, encode_lane_data, prepare_messages_storage_proof},
	BridgedChainOf, LaneIdOf, ThisChainOf,
};
use pallet_bridge_parachains::ParachainHeaders;
use pallet_transaction_storage::{
	AuthorizationExtent, Call as TxStorageCall, Config as TxStorageConfig, BAD_DATA_SIZE,
};
use runtime::{
	bridge_config::bp_people_polkadot, BuildStorage, Executive, Hash, Header, Runtime, RuntimeCall,
	RuntimeOrigin, SignedPayload, System, TxExtension, UncheckedExtrinsic,
};
use sp_consensus_grandpa::{AuthorityList, SetId};
use sp_core::{Encode, Pair};
use sp_keyring::{Sr25519Keyring, Sr25519Keyring as AccountKeyring};
use sp_runtime::{
	generic::Era,
	traits::{Header as _, SaturatedConversion},
	transaction_validity::{InvalidTransaction, TransactionValidityError},
	ApplyExtrinsicResult,
};
use sp_trie::{trie_types::TrieDBMutBuilderV1, LayoutV1, MemoryDB, TrieMut};

fn advance_block() {
	let current_number = System::block_number();
	if current_number > 0 {
		Executive::finalize_block();
	}
	let next_number = current_number + 1;
	let header = Header::new(
		next_number,
		Default::default(),
		Default::default(),
		Default::default(),
		Default::default(),
	);
	Executive::initialize_block(&header);

	let slot = runtime::Babe::current_slot();
	let now = slot.saturated_into::<u64>() * runtime::SLOT_DURATION;
	assert_ok!(runtime::Timestamp::set(RuntimeOrigin::none(), now));
}

pub fn run_test<T>(test: impl FnOnce() -> T) -> T {
	sp_tracing::try_init_simple();
	let mut t = frame_system::GenesisConfig::<Runtime>::default().build_storage().unwrap();
	pallet_relayer_set::GenesisConfig::<Runtime> {
		initial_relayers: vec![relayer_signer().into(), sudo_relayer_signer().into()],
	}
	.assimilate_storage(&mut t)
	.unwrap();
	pallet_bridge_grandpa::GenesisConfig::<Runtime> {
		owner: Some(bridge_owner_signer().to_account_id()),
		..Default::default()
	}
	.assimilate_storage(&mut t)
	.unwrap();
	pallet_sudo::GenesisConfig::<Runtime> { key: Some(sudo_relayer_signer().into()) }
		.assimilate_storage(&mut t)
		.unwrap();

	sp_io::TestExternalities::new(t).execute_with(test)
}

const POLKADOT_HEADER_NUMBER: bp_polkadot_core::BlockNumber = 100;
const PEOPLE_POLKADOT_HEADER_NUMBER: bp_people_polkadot::BlockNumber = 200;

#[derive(Clone, Copy)]
enum HeaderType {
	WithMessages,
	WithDeliveredMessages,
}

fn assert_ok_ok(apply_result: ApplyExtrinsicResult) {
	assert_ok!(apply_result);
	assert_ok!(apply_result.unwrap());
}

fn assert_ok_err(res: ApplyExtrinsicResult, expected: sp_runtime::DispatchError) {
	match res {
		Ok(Err(e)) => assert_eq!(e, expected),
		Ok(Ok(_)) => panic!("expected dispatch error, but call succeeded"),
		Err(e) => panic!("expected valid tx; got validity error: {e:?}"),
	}
}

fn sudo_relayer_signer() -> AccountKeyring {
	AccountKeyring::Alice
}

fn relayer_signer() -> AccountKeyring {
	AccountKeyring::Bob
}

fn non_relay_signer() -> AccountKeyring {
	AccountKeyring::Charlie
}

fn bridge_owner_signer() -> AccountKeyring {
	AccountKeyring::Bob
}

fn polkadot_initial_header() -> bp_polkadot_core::Header {
	bp_test_utils::test_header(POLKADOT_HEADER_NUMBER - 1)
}

fn polkadot_header(t: HeaderType) -> bp_polkadot_core::Header {
	let people_polkadot_head_storage_proof = people_polkadot_head_storage_proof(t);
	let state_root = people_polkadot_head_storage_proof.0;
	bp_test_utils::test_header_with_root(POLKADOT_HEADER_NUMBER, state_root)
}

fn polkadot_grandpa_justification(t: HeaderType) -> GrandpaJustification<bp_polkadot_core::Header> {
	bp_test_utils::make_default_justification(&polkadot_header(t))
}

fn polkadot_authority_set() -> AuthorityList {
	bp_test_utils::authority_list()
}

fn polkadot_authority_set_id() -> SetId {
	1
}

fn people_polkadot_head_storage_proof(t: HeaderType) -> (bp_polkadot_core::Hash, ParaHeadsProof) {
	let (state_root, proof, _) =
		bp_test_utils::prepare_parachain_heads_proof::<bp_polkadot_core::Header>(vec![(
			bp_people_polkadot::PEOPLE_POLKADOT_PARACHAIN_ID,
			ParaHead(people_polkadot_header(t).encode()),
		)]);
	(state_root, proof)
}

fn people_polkadot_header(t: HeaderType) -> bp_people_polkadot::Header {
	bp_test_utils::test_header_with_root(
		PEOPLE_POLKADOT_HEADER_NUMBER,
		match t {
			HeaderType::WithMessages => people_polkadot_message_storage_proof().0,
			HeaderType::WithDeliveredMessages => people_polkadot_message_delivery_storage_proof().0,
		},
	)
}

fn people_polkadot_message_delivery_storage_proof() -> (bp_people_polkadot::Hash, RawStorageProof) {
	let storage_key = bp_messages::storage_keys::inbound_lane_data_key(
		<BridgedChainOf<Runtime, WithPeoplePolkadotMessagesInstance>>::WITH_CHAIN_MESSAGES_PALLET_NAME,
		&XCM_LANE,
	)
	.0;
	let storage_value = InboundLaneData::<AccountId> {
		relayers: vec![UnrewardedRelayer {
			relayer: relayer_signer().into(),
			messages: DeliveredMessages { begin: 1, end: 1 },
		}]
		.into(),
		last_confirmed_nonce: 0,
		state: LaneState::Opened,
	}
	.encode();
	let mut root = Default::default();
	let mut mdb = MemoryDB::default();
	{
		let mut trie =
			TrieDBMutBuilderV1::<bp_people_polkadot::Hasher>::new(&mut mdb, &mut root).build();
		trie.insert(&storage_key, &storage_value).unwrap();
	}

	let storage_proof =
		record_all_trie_keys::<LayoutV1<bp_people_polkadot::Hasher>, _>(&mdb, &root).unwrap();

	(root, storage_proof)
}

fn people_polkadot_message_storage_proof() -> (bp_people_polkadot::Hash, RawStorageProof) {
	prepare_messages_storage_proof::<
		BridgedChainOf<Runtime, WithPeoplePolkadotMessagesInstance>,
		ThisChainOf<Runtime, WithPeoplePolkadotMessagesInstance>,
		LaneIdOf<Runtime, WithPeoplePolkadotMessagesInstance>,
	>(
		XCM_LANE,
		1..=1,
		None,
		UnverifiedStorageProofParams::default(),
		|_| vec![42],
		encode_all_messages,
		encode_lane_data,
		false,
		false,
	)
}

fn initialize_polkadot_grandpa_pallet() -> ApplyExtrinsicResult {
	construct_and_apply_extrinsic(
		bridge_owner_signer().pair(),
		RuntimeCall::BridgePolkadotGrandpa(pallet_bridge_grandpa::Call::initialize {
			init_data: InitializationData {
				header: Box::new(polkadot_initial_header()),
				authority_list: polkadot_authority_set(),
				set_id: polkadot_authority_set_id(),
				operating_mode: BasicOperatingMode::Normal,
			},
		}),
	)
}

fn submit_polkadot_header(signer: AccountKeyring, t: HeaderType) -> ApplyExtrinsicResult {
	construct_and_apply_extrinsic(
		signer.pair(),
		RuntimeCall::BridgePolkadotGrandpa(pallet_bridge_grandpa::Call::submit_finality_proof {
			finality_target: Box::new(polkadot_header(t)),
			justification: polkadot_grandpa_justification(t),
		}),
	)
}

fn submit_polkadot_people_hub_header(
	signer: AccountKeyring,
	t: HeaderType,
) -> ApplyExtrinsicResult {
	construct_and_apply_extrinsic(
		signer.pair(),
		RuntimeCall::BridgePolkadotParachains(
			pallet_bridge_parachains::Call::submit_parachain_heads {
				at_relay_block: (POLKADOT_HEADER_NUMBER, polkadot_header(t).hash()),
				parachains: vec![(
					bp_people_polkadot::PEOPLE_POLKADOT_PARACHAIN_ID.into(),
					people_polkadot_header(t).hash(),
				)],
				parachain_heads_proof: people_polkadot_head_storage_proof(t).1,
			},
		),
	)
}

fn emulate_sent_messages() {
	pallet_bridge_messages::OutboundLanes::<Runtime, WithPeoplePolkadotMessagesInstance>::insert(
		XCM_LANE,
		OutboundLaneData {
			oldest_unpruned_nonce: 1,
			latest_received_nonce: 0,
			latest_generated_nonce: 1,
			state: LaneState::Opened,
		},
	);
}

fn construct_extrinsic(
	sender: sp_core::sr25519::Pair,
	call: RuntimeCall,
) -> Result<UncheckedExtrinsic, sp_runtime::transaction_validity::TransactionValidityError> {
	let account_id = sp_runtime::AccountId32::from(sender.public());
	frame_system::BlockHash::<Runtime>::insert(0, Hash::default());
	let tx_ext: TxExtension = (
		frame_system::CheckNonZeroSender::<Runtime>::new(),
		frame_system::CheckSpecVersion::<Runtime>::new(),
		frame_system::CheckTxVersion::<Runtime>::new(),
		frame_system::CheckGenesis::<Runtime>::new(),
		frame_system::CheckEra::<Runtime>::from(Era::immortal()),
		frame_system::CheckNonce::<Runtime>::from(
			frame_system::Pallet::<Runtime>::account(&account_id).nonce,
		),
		frame_system::CheckWeight::<Runtime>::new(),
		runtime::ValidateSigned,
		runtime::BridgeRejectObsoleteHeadersAndMessages,
	);
	let payload = SignedPayload::new(call.clone(), tx_ext.clone())?;
	let signature = payload.using_encoded(|e| sender.sign(e));
	Ok(UncheckedExtrinsic::new_signed(
		call,
		account_id.into(),
		runtime::Signature::Sr25519(signature),
		tx_ext,
	))
}

fn construct_and_apply_extrinsic(
	account: sp_core::sr25519::Pair,
	call: RuntimeCall,
) -> ApplyExtrinsicResult {
	let dispatch_info = call.get_dispatch_info();
	let xt = construct_extrinsic(account, call)?;
	let xt_len = xt.encode().len();
	log::info!(
		"Applying extrinsic: class={:?} pays_fee={:?} weight={:?} encoded_len={} bytes",
		dispatch_info.class,
		dispatch_info.pays_fee,
		dispatch_info.total_weight(),
		xt_len
	);
	Executive::apply_extrinsic(xt)
}

#[test]
fn transaction_storage_runtime_sizes() {
	sp_tracing::try_init_simple();
	sp_io::TestExternalities::new(
		runtime::RuntimeGenesisConfig::default().build_storage().unwrap(),
	)
	.execute_with(|| {
		advance_block();

		// prepare data
		let account = Sr25519Keyring::Alice;
		let who: runtime::AccountId = account.to_account_id();
		#[allow(clippy::identity_op)]
		let sizes: [usize; 5] = [
			2000,            // 2 KB
			1 * 1024 * 1024, // 1 MB
			4 * 1024 * 1024, // 4 MB
			6 * 1024 * 1024, // 6 MB
			8 * 1024 * 1024, // 8 MB
		];
		let total_bytes: u64 = sizes.iter().map(|s| *s as u64).sum();

		// authorize
		assert_ok!(runtime::TransactionStorage::authorize_account(
			runtime::RuntimeOrigin::root(),
			who.clone(),
			sizes.len() as u32,
			total_bytes,
		));
		assert_eq!(
			runtime::TransactionStorage::account_authorization_extent(who.clone()),
			AuthorizationExtent { transactions: sizes.len() as u32, bytes: total_bytes },
		);

		// store data
		for (index, size) in sizes.into_iter().enumerate() {
			log::info!("Storing data with size: {size} and index: {index}");
			advance_block();
			let res = construct_and_apply_extrinsic(
				account.pair(),
				RuntimeCall::TransactionStorage(TxStorageCall::<runtime::Runtime>::store {
					data: vec![0u8; size],
				}),
			);
			assert_ok_ok(res);
		}
		assert_eq!(
			runtime::TransactionStorage::account_authorization_extent(who.clone()),
			AuthorizationExtent { transactions: 0, bytes: 0 },
		);

		// (MaxTransactionSize+1) should exceed MaxTransactionSize and fail
		let oversized: u64 =
			(<<runtime::Runtime as TxStorageConfig>::MaxTransactionSize as Get<u32>>::get() + 1)
				.into();
		advance_block();
		assert_ok!(runtime::TransactionStorage::authorize_account(
			runtime::RuntimeOrigin::root(),
			who.clone(),
			1,
			oversized,
		));
		assert_eq!(
			runtime::TransactionStorage::account_authorization_extent(who),
			AuthorizationExtent { transactions: 1_u32, bytes: oversized },
		);
		assert_eq!(
			construct_and_apply_extrinsic(
				account.pair(),
				RuntimeCall::TransactionStorage(TxStorageCall::<runtime::Runtime>::store {
					data: vec![0u8; oversized as usize]
				})
			),
			Err(BAD_DATA_SIZE.into())
		);
	});
}

#[test]
fn only_relayer_may_submit_polkadot_headers() {
	run_test(|| {
		assert_ok_ok(initialize_polkadot_grandpa_pallet());

		assert_eq!(BridgePolkadotGrandpa::best_finalized(), Some(polkadot_initial_header().id()));

		// Non-relayer may not submit Polkadot headers
		// can't use assert_noop here, because we need to mutate storage inside
		// the `construct_and_apply_extrinsic`
		assert_eq!(
			submit_polkadot_header(non_relay_signer(), HeaderType::WithMessages),
			// no providers or sufficients
			Err(TransactionValidityError::Invalid(InvalidTransaction::Payment))
		);
		assert_eq!(BridgePolkadotGrandpa::best_finalized(), Some(polkadot_initial_header().id()));

		// Relayer may submit Polkadot headers
		assert_ok_ok(submit_polkadot_header(relayer_signer(), HeaderType::WithMessages));
		assert_eq!(
			BridgePolkadotGrandpa::best_finalized(),
			Some(polkadot_header(HeaderType::WithMessages).id())
		);
	});
}

#[test]
fn only_relayer_may_submit_polkadot_people_hub_headers() {
	run_test(|| {
		assert_ok_ok(initialize_polkadot_grandpa_pallet());
		assert_ok_ok(submit_polkadot_header(relayer_signer(), HeaderType::WithMessages));

		assert_eq!(
			BridgePolkadotGrandpa::finalized_header_state_root(
				people_polkadot_header(HeaderType::WithMessages).hash()
			),
			None,
		);

		// Non-relayer may NOT submit Polkadot BH headers
		// can't use assert_noop here, because we need to mutate storage inside
		// the `construct_and_apply_extrinsic`
		assert_eq!(
			submit_polkadot_people_hub_header(non_relay_signer(), HeaderType::WithMessages),
			// no providers or sufficients
			Err(TransactionValidityError::Invalid(InvalidTransaction::Payment)),
		);
		assert_eq!(
			ParachainHeaders::<
				Runtime,
				WithPolkadotBridgeParachainsInstance,
				bp_people_polkadot::PeoplePolkadot,
			>::finalized_header_state_root(
				people_polkadot_header(HeaderType::WithMessages).hash()
			),
			None
		);

		// Relayer may submit Polkadot BH headers
		assert_ok_ok(submit_polkadot_people_hub_header(relayer_signer(), HeaderType::WithMessages));
		assert_eq!(
			ParachainHeaders::<
				Runtime,
				WithPolkadotBridgeParachainsInstance,
				bp_people_polkadot::PeoplePolkadot,
			>::finalized_header_state_root(
				people_polkadot_header(HeaderType::WithMessages).hash()
			),
			Some(*people_polkadot_header(HeaderType::WithMessages).state_root())
		);
	});
}

#[test]
fn only_relayer_may_deliver_messages_from_polkadot_bridge_hub() {
	run_test(|| {
		assert_ok_ok(initialize_polkadot_grandpa_pallet());
		assert_ok_ok(submit_polkadot_header(relayer_signer(), HeaderType::WithMessages));
		assert_ok_ok(submit_polkadot_people_hub_header(relayer_signer(), HeaderType::WithMessages));
		assert!(BridgePolkadotMessages::inbound_lane_data(XCM_LANE).is_none());

		// TODO: finish
		// // Non-relayer may NOT deliver messages from Polkadot BH
		// assert_eq!(
		// 	submit_messages_from_polkadot_bridge_hub(non_relay_signer()),
		// 	Err(TransactionValidityError::Invalid(InvalidTransaction::BadSigner)),
		// );
		// assert!(BridgePolkadotMessages::inbound_lane_data(XCM_LANE).relayers.is_empty());
		//
		// // Relayer may deliver messages from Polkadot BH
		// assert_ok_ok(submit_messages_from_polkadot_bridge_hub(relayer_signer()));
		// assert!(!BridgePolkadotMessages::inbound_lane_data(XCM_LANE).relayers.is_empty());
	});
}

#[test]
fn only_relayer_may_deliver_confirmations_from_polkadot_bridge_hub() {
	run_test(|| {
		assert_ok_ok(initialize_polkadot_grandpa_pallet());
		assert_ok_ok(submit_polkadot_header(relayer_signer(), HeaderType::WithDeliveredMessages));
		assert_ok_ok(submit_polkadot_people_hub_header(
			relayer_signer(),
			HeaderType::WithDeliveredMessages,
		));
		emulate_sent_messages();

		assert_eq!(
			BridgePolkadotMessages::outbound_lane_data(XCM_LANE)
				.unwrap()
				.latest_received_nonce,
			0
		);

		// TODO: finish
		// // Non-relayer may NOT deliver confirmations from Polkadot BH
		// assert_eq!(
		// 	submit_confirmations_from_polkadot_bridge_hub(non_relay_signer()),
		// 	Err(TransactionValidityError::Invalid(InvalidTransaction::BadSigner)),
		// );
		// assert_eq!(BridgePolkadotMessages::outbound_lane_data(XCM_LANE).latest_received_nonce,
		// 0);
		//
		// // Relayer may deliver confirmations from Polkadot BH
		// assert_ok_ok(submit_confirmations_from_polkadot_bridge_hub(relayer_signer()));
		// assert_ne!(BridgePolkadotMessages::outbound_lane_data(XCM_LANE).latest_received_nonce,
		// 0);
	});
}

fn test_sudo_can_execute_authorize_upgrade(system_call: RuntimeCall) {
	run_test(|| {
		assert!(runtime::System::authorized_upgrade().is_none());

		let sudo_signer = sudo_relayer_signer();

		let call_wrapped_in_sudo =
			RuntimeCall::Sudo(pallet_sudo::Call::sudo { call: Box::new(system_call.clone()) });

		assert_ok_ok(construct_and_apply_extrinsic(sudo_signer.pair(), call_wrapped_in_sudo));

		assert!(runtime::System::authorized_upgrade().is_some());
	});
}

#[test]
fn sudo_can_execute_authorize_upgrade() {
	let wasm_hash: sp_core::H256 = [0xFFu8; 32].into();
	run_test(|| {
		test_sudo_can_execute_authorize_upgrade(RuntimeCall::System(
			runtime::SystemCall::authorize_upgrade { code_hash: wasm_hash },
		));
	});
}

#[test]
fn sudo_can_execute_authorize_upgradewithout_checks() {
	let wasm_hash: sp_core::H256 = [0xFFu8; 32].into();
	run_test(|| {
		test_sudo_can_execute_authorize_upgrade(RuntimeCall::System(
			runtime::SystemCall::authorize_upgrade_without_checks { code_hash: wasm_hash },
		));
	});
}

fn test_non_sudo_cannot_execute_authorize_upgrade(system_call: RuntimeCall) {
	run_test(|| {
		assert!(runtime::System::authorized_upgrade().is_none());

		let non_sudo_signer = relayer_signer();

		let call_wrapped_in_sudo =
			RuntimeCall::Sudo(pallet_sudo::Call::sudo { call: Box::new(system_call.clone()) });

		assert_ok_err(
			construct_and_apply_extrinsic(non_sudo_signer.pair(), call_wrapped_in_sudo),
			pallet_sudo::Error::<Runtime>::RequireSudo.into(),
		);

		assert!(runtime::System::authorized_upgrade().is_none());
	});
}

#[test]
fn non_sudo_cannot_execute_authorize_upgrade() {
	let wasm_hash: sp_core::H256 = [0xFFu8; 32].into();
	run_test(|| {
		test_non_sudo_cannot_execute_authorize_upgrade(RuntimeCall::System(
			runtime::SystemCall::authorize_upgrade { code_hash: wasm_hash },
		));
	});
}

#[test]
fn non_sudo_cannot_execute_authorize_upgrade_without_checks() {
	let wasm_hash: sp_core::H256 = [0xFFu8; 32].into();
	run_test(|| {
		test_non_sudo_cannot_execute_authorize_upgrade(RuntimeCall::System(
			runtime::SystemCall::authorize_upgrade_without_checks { code_hash: wasm_hash },
		));
	});
}

fn test_sudo_proxy_authorize_upgrade(system_call: RuntimeCall) {
	let sudo_signer = sudo_relayer_signer();
	let non_sudo_signer = relayer_signer();

	let add_proxy_call = RuntimeCall::Proxy(pallet_proxy::Call::add_proxy {
		delegate: sp_runtime::MultiAddress::Id(non_sudo_signer.to_account_id()),
		proxy_type: Default::default(),
		delay: 0,
	});
	assert_ok_ok(construct_and_apply_extrinsic(sudo_signer.pair(), add_proxy_call));

	let call_wrapped_in_sudo =
		RuntimeCall::Sudo(pallet_sudo::Call::sudo { call: Box::new(system_call) });

	let sudo_wrapped_in_proxy_call = RuntimeCall::Proxy(pallet_proxy::Call::proxy {
		real: sp_runtime::MultiAddress::Id(sudo_signer.to_account_id()),
		force_proxy_type: None,
		call: Box::new(call_wrapped_in_sudo),
	});

	assert_ok_ok(construct_and_apply_extrinsic(non_sudo_signer.pair(), sudo_wrapped_in_proxy_call));

	assert!(runtime::System::authorized_upgrade().is_some());
}

#[test]
fn sudo_can_add_proxy_then_proxy_executes_authorize_upgrade() {
	let wasm_hash: sp_core::H256 = [0xFFu8; 32].into();
	run_test(|| {
		test_sudo_proxy_authorize_upgrade(RuntimeCall::System(
			runtime::SystemCall::authorize_upgrade { code_hash: wasm_hash },
		));
	});
}

#[test]
fn sudo_can_add_proxy_then_proxy_executes_authorize_upgrade_without_checks() {
	let wasm_hash: sp_core::H256 = [0xFFu8; 32].into();
	run_test(|| {
		test_sudo_proxy_authorize_upgrade(RuntimeCall::System(
			runtime::SystemCall::authorize_upgrade_without_checks { code_hash: wasm_hash },
		));
	});
}

#[test]
fn sudo_can_add_non_relayer_proxy_but_proxy_still_cannot_execute() {
	run_test(|| {
		assert!(runtime::System::authorized_upgrade().is_none());

		let sudo_signer = sudo_relayer_signer();
		let non_relayer_signer = non_relay_signer();

		let wasm_hash = runtime::System::block_hash(0);

		let add_proxy_call = RuntimeCall::Proxy(pallet_proxy::Call::add_proxy {
			delegate: sp_runtime::MultiAddress::Id(non_relayer_signer.to_account_id()),
			proxy_type: Default::default(),
			delay: 0,
		});
		assert_ok_ok(construct_and_apply_extrinsic(sudo_signer.pair(), add_proxy_call));

		let call =
			RuntimeCall::System(runtime::SystemCall::authorize_upgrade { code_hash: wasm_hash });
		let call_wrapped_in_sudo =
			RuntimeCall::Sudo(pallet_sudo::Call::sudo { call: Box::new(call.clone()) });

		let sudo_wrapped_in_proxy_call = RuntimeCall::Proxy(pallet_proxy::Call::proxy {
			real: sp_runtime::MultiAddress::Id(sudo_signer.to_account_id()),
			force_proxy_type: None,
			call: Box::new(call_wrapped_in_sudo),
		});

		assert_eq!(
			construct_and_apply_extrinsic(non_relayer_signer.pair(), sudo_wrapped_in_proxy_call,),
			Err(TransactionValidityError::Invalid(InvalidTransaction::Payment))
		);

		assert!(runtime::System::authorized_upgrade().is_none());
	});
}

#[test]
fn can_add_up_to_max_number_of_proxies_and_fail_beyond() {
	run_test(|| {
		let sudo_signer = sudo_relayer_signer();

		let max_proxies: u32 = <Runtime as pallet_proxy::Config>::MaxProxies::get();

		let delegates: Vec<runtime::AccountId> = (1..=max_proxies)
			.map(|i| {
				let bytes = [i as u8; 32];
				bytes.into()
			})
			.collect();

		for delegate in &delegates {
			let add_proxy_call = RuntimeCall::Proxy(pallet_proxy::Call::add_proxy {
				delegate: sp_runtime::MultiAddress::Id(delegate.clone()),
				proxy_type: Default::default(),
				delay: 0,
			});

			assert_ok_ok(construct_and_apply_extrinsic(sudo_signer.pair(), add_proxy_call));
		}

		let extra_account: runtime::AccountId = [0xFFu8; 32].into();
		let extra_call = RuntimeCall::Proxy(pallet_proxy::Call::add_proxy {
			delegate: sp_runtime::MultiAddress::Id(extra_account.clone()),
			proxy_type: Default::default(),
			delay: 0,
		});

		assert_ok_err(
			construct_and_apply_extrinsic(sudo_signer.pair(), extra_call),
			pallet_proxy::Error::<Runtime>::TooMany.into(),
		);
	});
}

#[test]
fn sudo_executes_authorize_upgrade_without_checks_and_non_sudo_apply_it() {
	run_test(|| {
		assert!(runtime::System::authorized_upgrade().is_none());

		let sudo_signer = sudo_relayer_signer();
		let non_sudo_signer = relayer_signer();

		let current_wasm =
			sp_io::storage::get(b":code").expect("runtime code must exist in :code storage key");
		let wasm_hash: runtime::Hash = sp_io::hashing::blake2_256(&current_wasm).into();

		let authorize_call =
			RuntimeCall::System(runtime::SystemCall::authorize_upgrade_without_checks {
				code_hash: wasm_hash,
			});
		let sudo_wrapped =
			RuntimeCall::Sudo(pallet_sudo::Call::sudo { call: Box::new(authorize_call.clone()) });

		assert_ok_ok(construct_and_apply_extrinsic(sudo_signer.pair(), sudo_wrapped));

		assert!(runtime::System::authorized_upgrade().is_some());

		let apply_call = RuntimeCall::System(runtime::SystemCall::apply_authorized_upgrade {
			code: current_wasm.to_vec(),
		});

		assert_ok_ok(construct_and_apply_extrinsic(non_sudo_signer.pair(), apply_call));
	});
}

#[test]
fn sudo_executes_authorize_upgrade_without_checks_with_wrong_hash_and_non_sudo_cannot_apply_it() {
	run_test(|| {
		assert!(runtime::System::authorized_upgrade().is_none());

		let sudo_signer = sudo_relayer_signer();
		let non_sudo_signer = relayer_signer();

		let current_wasm =
			sp_io::storage::get(b":code").expect("runtime code must exist in :code storage key");
		let wrong_hash: runtime::Hash = [0xFFu8; 32].into();

		let authorize_call =
			RuntimeCall::System(runtime::SystemCall::authorize_upgrade_without_checks {
				code_hash: wrong_hash,
			});
		let sudo_wrapped =
			RuntimeCall::Sudo(pallet_sudo::Call::sudo { call: Box::new(authorize_call.clone()) });

		assert_ok_ok(construct_and_apply_extrinsic(sudo_signer.pair(), sudo_wrapped));

		assert!(runtime::System::authorized_upgrade().is_some());

		let apply_call = RuntimeCall::System(runtime::SystemCall::apply_authorized_upgrade {
			code: current_wasm.to_vec(),
		});

		assert_ok_err(
			construct_and_apply_extrinsic(non_sudo_signer.pair(), apply_call),
			frame_system::Error::<Runtime>::Unauthorized.into(),
		);
	});
}

#[test]
fn sudo_executes_set_code_without_checks_is_success() {
	run_test(|| {
		let sudo_signer = sudo_relayer_signer();

		let current_wasm =
			sp_io::storage::get(b":code").expect("runtime code must exist in :code storage key");

		let set_code_call = RuntimeCall::System(runtime::SystemCall::set_code_without_checks {
			code: current_wasm.to_vec(),
		});
		let sudo_wrapped =
			RuntimeCall::Sudo(pallet_sudo::Call::sudo { call: Box::new(set_code_call.clone()) });

		assert_ok_ok(construct_and_apply_extrinsic(sudo_signer.pair(), sudo_wrapped));
	});
}

#[test]
fn sudo_kill_works() {
	run_test(|| {
		let sudo_signer = sudo_relayer_signer();

		// Sudo works
		let sudo_test_call =
			RuntimeCall::System(runtime::SystemCall::authorize_upgrade_without_checks {
				code_hash: [0xFFu8; 32].into(),
			});
		let sudo_wrapped =
			RuntimeCall::Sudo(pallet_sudo::Call::sudo { call: Box::new(sudo_test_call.clone()) });
		assert_ok_ok(construct_and_apply_extrinsic(sudo_signer.pair(), sudo_wrapped.clone()));

		// Remove sudo key
		let remove_key = RuntimeCall::Sudo(pallet_sudo::Call::remove_key {});
		assert_ok_ok(construct_and_apply_extrinsic(sudo_signer.pair(), remove_key));

		// Sudo no longer works
		assert_ok_err(
			construct_and_apply_extrinsic(sudo_signer.pair(), sudo_wrapped),
			pallet_sudo::Error::<Runtime>::RequireSudo.into(),
		);
	});
}
