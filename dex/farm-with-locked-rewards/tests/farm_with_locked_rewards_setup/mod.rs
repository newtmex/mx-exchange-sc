use common_structs::FarmTokenAttributes;
use config::ConfigModule;
use elrond_wasm::{
    storage::mappers::StorageTokenWrapper,
    types::{Address, BigInt, EsdtLocalRole, MultiValueEncoded},
};
use elrond_wasm_debug::{
    managed_address, managed_biguint, managed_token_id, rust_biguint,
    testing_framework::{BlockchainStateWrapper, ContractObjWrapper},
    DebugApi,
};

mod fees_collector_mock;
use fees_collector_mock::*;

use elrond_wasm_modules::pause::PauseModule;
use energy_factory_mock::EnergyFactoryMock;
use energy_query::{Energy, EnergyQueryModule};
use farm_boosted_yields::FarmBoostedYieldsModule;
use farm_token::FarmTokenModule;
use farm_with_locked_rewards::Farm;
use locking_module::lock_with_energy_module::LockWithEnergyModule;
use pausable::{PausableModule, State};
use sc_whitelist_module::SCWhitelistModule;
use simple_lock::locked_token::LockedTokenModule;
use simple_lock_energy::lock_options::LockOptionsModule;
use simple_lock_energy::SimpleLockEnergy;

pub static REWARD_TOKEN_ID: &[u8] = b"MEX-123456";
pub static LOCKED_REWARD_TOKEN_ID: &[u8] = b"LOCKED-123456";
pub static LEGACY_LOCKED_TOKEN_ID: &[u8] = b"LEGACY-123456";
pub static FARMING_TOKEN_ID: &[u8] = b"LPTOK-123456";
pub static FARM_TOKEN_ID: &[u8] = b"FARM-123456";
const DIV_SAFETY: u64 = 1_000_000_000_000;
const PER_BLOCK_REWARD_AMOUNT: u64 = 1_000;
const FARMING_TOKEN_BALANCE: u64 = 100_000_000;
pub const BOOSTED_YIELDS_PERCENTAGE: u64 = 2_500; // 25%

pub const EPOCHS_IN_YEAR: u64 = 365;

pub const MIN_PENALTY_PERCENTAGE: u16 = 1; // 0.01%
pub const MAX_PENALTY_PERCENTAGE: u16 = 10_000; // 100%
pub const FEES_BURN_PERCENTAGE: u16 = 5_000; // 50%
pub static LOCK_OPTIONS: &[u64] = &[EPOCHS_IN_YEAR, 2 * EPOCHS_IN_YEAR, 4 * EPOCHS_IN_YEAR];

pub struct FarmSetup<FarmObjBuilder, EnergyFactoryBuilder, SimpleLockEnergyBuilder>
where
    FarmObjBuilder: 'static + Copy + Fn() -> farm_with_locked_rewards::ContractObj<DebugApi>,
    EnergyFactoryBuilder: 'static + Copy + Fn() -> energy_factory_mock::ContractObj<DebugApi>,
    SimpleLockEnergyBuilder: 'static + Copy + Fn() -> simple_lock_energy::ContractObj<DebugApi>,
{
    pub b_mock: BlockchainStateWrapper,
    pub owner: Address,
    pub first_user: Address,
    pub second_user: Address,
    pub last_farm_token_nonce: u64,
    pub farm_wrapper:
        ContractObjWrapper<farm_with_locked_rewards::ContractObj<DebugApi>, FarmObjBuilder>,
    pub energy_factory_wrapper:
        ContractObjWrapper<energy_factory_mock::ContractObj<DebugApi>, EnergyFactoryBuilder>,
    pub simple_lock_energy_wrapper:
        ContractObjWrapper<simple_lock_energy::ContractObj<DebugApi>, SimpleLockEnergyBuilder>,
}

impl<FarmObjBuilder, EnergyFactoryBuilder, SimpleLockEnergyBuilder>
    FarmSetup<FarmObjBuilder, EnergyFactoryBuilder, SimpleLockEnergyBuilder>
where
    FarmObjBuilder: 'static + Copy + Fn() -> farm_with_locked_rewards::ContractObj<DebugApi>,
    EnergyFactoryBuilder: 'static + Copy + Fn() -> energy_factory_mock::ContractObj<DebugApi>,
    SimpleLockEnergyBuilder: 'static + Copy + Fn() -> simple_lock_energy::ContractObj<DebugApi>,
{
    pub fn new(
        farm_builder: FarmObjBuilder,
        energy_factory_builder: EnergyFactoryBuilder,
        simple_lock_energy_builder: SimpleLockEnergyBuilder,
    ) -> Self {
        let rust_zero = rust_biguint!(0);
        let mut b_mock = BlockchainStateWrapper::new();
        let owner = b_mock.create_user_account(&rust_zero);
        let first_user = b_mock.create_user_account(&rust_zero);
        let second_user = b_mock.create_user_account(&rust_zero);
        let farm_wrapper = b_mock.create_sc_account(
            &rust_zero,
            Some(&owner),
            farm_builder,
            "farm-with-locked-rewards.wasm",
        );
        let energy_factory_wrapper = b_mock.create_sc_account(
            &rust_zero,
            Some(&owner),
            energy_factory_builder,
            "energy_factory.wasm",
        );
        let simple_lock_energy_wrapper = b_mock.create_sc_account(
            &rust_zero,
            Some(&owner),
            simple_lock_energy_builder,
            "simple-lock-energy.wasm",
        );
        let fees_collector_mock = b_mock.create_sc_account(
            &rust_zero,
            Some(&owner),
            FeesCollectorMock::new,
            "fees collector mock",
        );

        b_mock
            .execute_tx(&owner, &simple_lock_energy_wrapper, &rust_zero, |sc| {
                let mut lock_options = MultiValueEncoded::new();
                for option in LOCK_OPTIONS {
                    lock_options.push(*option);
                }

                sc.init(
                    managed_token_id!(REWARD_TOKEN_ID),
                    managed_token_id!(LEGACY_LOCKED_TOKEN_ID),
                    MIN_PENALTY_PERCENTAGE,
                    MAX_PENALTY_PERCENTAGE,
                    FEES_BURN_PERCENTAGE,
                    managed_address!(fees_collector_mock.address_ref()),
                    managed_address!(fees_collector_mock.address_ref()),
                    lock_options,
                );

                assert_eq!(sc.max_lock_option().get(), *LOCK_OPTIONS.last().unwrap());

                sc.locked_token()
                    .set_token_id(&managed_token_id!(LOCKED_REWARD_TOKEN_ID));
                sc.set_paused(false);
            })
            .assert_ok();

        b_mock
            .execute_tx(&owner, &farm_wrapper, &rust_zero, |sc| {
                let reward_token_id = managed_token_id!(REWARD_TOKEN_ID);
                let farming_token_id = managed_token_id!(FARMING_TOKEN_ID);
                let division_safety_constant = managed_biguint!(DIV_SAFETY);
                let pair_address = managed_address!(&Address::zero());

                sc.init(
                    reward_token_id,
                    farming_token_id,
                    division_safety_constant,
                    pair_address,
                    managed_address!(&owner),
                    MultiValueEncoded::new(),
                );

                let farm_token_id = managed_token_id!(FARM_TOKEN_ID);
                sc.farm_token().set_token_id(&farm_token_id);
                sc.set_locking_sc_address(managed_address!(
                    simple_lock_energy_wrapper.address_ref()
                ));
                sc.set_lock_epochs(EPOCHS_IN_YEAR);

                // sc.base_asset_token_id(managed_token_id!(REWARD_TOKEN_ID));

                //TODO - change to proxy deployer
                sc.add_sc_address_to_whitelist(managed_address!(&first_user));
                sc.add_sc_address_to_whitelist(managed_address!(&second_user));

                sc.per_block_reward_amount()
                    .set(&managed_biguint!(PER_BLOCK_REWARD_AMOUNT));

                sc.state().set(State::Active);
                sc.produce_rewards_enabled().set(true);
                sc.set_energy_factory_address(managed_address!(
                    energy_factory_wrapper.address_ref()
                ));
            })
            .assert_ok();

        let farm_token_roles = [
            EsdtLocalRole::NftCreate,
            EsdtLocalRole::NftAddQuantity,
            EsdtLocalRole::NftBurn,
        ];
        b_mock.set_esdt_local_roles(
            farm_wrapper.address_ref(),
            FARM_TOKEN_ID,
            &farm_token_roles[..],
        );

        let farming_token_roles = [EsdtLocalRole::Burn];
        b_mock.set_esdt_local_roles(
            farm_wrapper.address_ref(),
            FARMING_TOKEN_ID,
            &farming_token_roles[..],
        );

        let reward_mint_token_roles = [EsdtLocalRole::Mint];
        b_mock.set_esdt_local_roles(
            farm_wrapper.address_ref(),
            REWARD_TOKEN_ID,
            &reward_mint_token_roles[..],
        );

        let reward_burn_token_roles = [EsdtLocalRole::Burn];
        b_mock.set_esdt_local_roles(
            simple_lock_energy_wrapper.address_ref(),
            REWARD_TOKEN_ID,
            &reward_burn_token_roles[..],
        );

        let locked_reward_token_roles = [
            EsdtLocalRole::NftCreate,
            EsdtLocalRole::NftAddQuantity,
            EsdtLocalRole::NftBurn,
            EsdtLocalRole::Transfer,
        ];
        b_mock.set_esdt_local_roles(
            simple_lock_energy_wrapper.address_ref(),
            LOCKED_REWARD_TOKEN_ID,
            &locked_reward_token_roles[..],
        );

        b_mock.set_esdt_balance(
            &first_user,
            FARMING_TOKEN_ID,
            &rust_biguint!(FARMING_TOKEN_BALANCE),
        );
        b_mock.set_esdt_balance(
            &second_user,
            FARMING_TOKEN_ID,
            &rust_biguint!(FARMING_TOKEN_BALANCE),
        );

        b_mock
            .execute_tx(&owner, &simple_lock_energy_wrapper, &rust_zero, |sc| {
                sc.sc_whitelist_addresses()
                    .add(&managed_address!(farm_wrapper.address_ref()));
            })
            .assert_ok();

        FarmSetup {
            b_mock,
            owner,
            first_user,
            second_user,
            last_farm_token_nonce: 0,
            farm_wrapper,
            energy_factory_wrapper,
            simple_lock_energy_wrapper,
        }
    }

    pub fn set_user_energy(
        &mut self,
        user: &Address,
        energy: u64,
        last_update_epoch: u64,
        locked_tokens: u64,
    ) {
        self.b_mock
            .execute_tx(
                &self.owner,
                &self.energy_factory_wrapper,
                &rust_biguint!(0),
                |sc| {
                    sc.user_energy(&managed_address!(user)).set(&Energy::new(
                        BigInt::from(managed_biguint!(energy)),
                        last_update_epoch,
                        managed_biguint!(locked_tokens),
                    ));
                },
            )
            .assert_ok();
    }

    pub fn set_boosted_yields_rewards_percentage(&mut self, percentage: u64) {
        self.b_mock
            .execute_tx(&self.owner, &self.farm_wrapper, &rust_biguint!(0), |sc| {
                sc.set_boosted_yields_rewards_percentage(percentage);
            })
            .assert_ok();
    }

    pub fn enter_farm(&mut self, user: &Address, farming_token_amount: u64) {
        self.last_farm_token_nonce += 1;

        let expected_farm_token_nonce = self.last_farm_token_nonce;
        self.b_mock
            .execute_esdt_transfer(
                user,
                &self.farm_wrapper,
                FARMING_TOKEN_ID,
                0,
                &rust_biguint!(farming_token_amount),
                |sc| {
                    let out_farm_token = sc.enter_farm_endpoint();
                    assert_eq!(
                        out_farm_token.token_identifier,
                        managed_token_id!(FARM_TOKEN_ID)
                    );
                    assert_eq!(out_farm_token.token_nonce, expected_farm_token_nonce);
                    assert_eq!(
                        out_farm_token.amount,
                        managed_biguint!(farming_token_amount)
                    );
                },
            )
            .assert_ok();
    }

    pub fn calculate_rewards(
        &mut self,
        user: &Address,
        farm_token_nonce: u64,
        farm_token_amount: u64,
        attributes: FarmTokenAttributes<DebugApi>,
    ) -> u64 {
        let mut result = 0;
        self.b_mock
            .execute_query(&self.farm_wrapper, |sc| {
                let result_managed = sc.calculate_rewards_for_given_position(
                    managed_address!(user),
                    farm_token_nonce,
                    managed_biguint!(farm_token_amount),
                    attributes,
                );
                result = result_managed.to_u64().unwrap();
            })
            .assert_ok();

        result
    }

    pub fn claim_rewards(
        &mut self,
        user: &Address,
        farm_token_nonce: u64,
        farm_token_amount: u64,
    ) -> u64 {
        self.last_farm_token_nonce += 1;

        let expected_farm_token_nonce = self.last_farm_token_nonce;
        let mut result = 0;
        self.b_mock
            .execute_esdt_transfer(
                user,
                &self.farm_wrapper,
                FARM_TOKEN_ID,
                farm_token_nonce,
                &rust_biguint!(farm_token_amount),
                |sc| {
                    let (out_farm_token, out_reward_token) =
                        sc.claim_rewards_endpoint().into_tuple();
                    assert_eq!(
                        out_farm_token.token_identifier,
                        managed_token_id!(FARM_TOKEN_ID)
                    );
                    assert_eq!(out_farm_token.token_nonce, expected_farm_token_nonce);
                    assert_eq!(out_farm_token.amount, managed_biguint!(farm_token_amount));

                    if out_reward_token.amount > 0 {
                        assert_eq!(
                            out_reward_token.token_identifier,
                            managed_token_id!(LOCKED_REWARD_TOKEN_ID)
                        );
                        assert_eq!(out_reward_token.token_nonce, 1);
                    } else {
                        assert_eq!(
                            out_reward_token.token_identifier,
                            managed_token_id!(REWARD_TOKEN_ID)
                        );
                        assert_eq!(out_reward_token.token_nonce, 0);
                    }

                    result = out_reward_token.amount.to_u64().unwrap();
                },
            )
            .assert_ok();

        result
    }

    pub fn exit_farm(&mut self, user: &Address, farm_token_nonce: u64, farm_token_amount: u64) {
        self.b_mock
            .execute_esdt_transfer(
                user,
                &self.farm_wrapper,
                FARM_TOKEN_ID,
                farm_token_nonce,
                &rust_biguint!(farm_token_amount),
                |sc| {
                    let _ = sc.exit_farm_endpoint();
                },
            )
            .assert_ok();
    }
}
