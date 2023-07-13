#![no_std]

multiversx_sc::imports!();

pub mod caller_check;
pub mod configurable;
mod errors;
pub mod events;
pub mod proposal;
pub mod proposal_storage;
pub mod views;

use proposal::*;
use proposal_storage::VoteType;
use weekly_rewards_splitting::events::Week;
use weekly_rewards_splitting::global_info::ProxyTrait as _;

use crate::errors::*;
use crate::proposal_storage::ProposalVotes;

const MAX_GAS_LIMIT_PER_BLOCK: u64 = 600_000_000;
const FULL_PERCENTAGE: u64 = 10_000;
static ALREADY_VOTED_ERR_MSG: &[u8] = b"Already voted for this proposal";

/// An empty contract. To be used as a template when starting a new contract from scratch.
#[multiversx_sc::contract]
pub trait GovernanceV2:
    configurable::ConfigurablePropertiesModule
    + events::EventsModule
    + proposal_storage::ProposalStorageModule
    + caller_check::CallerCheckModule
    + views::ViewsModule
    + energy_query::EnergyQueryModule
    + permissions_module::PermissionsModule
{
    /// - `min_energy_for_propose` - the minimum energy required for submitting a proposal
    /// - `min_fee_for_propose` - the minimum fee required for submitting a proposal
    /// - `quorum` - the minimum number of (`votes` minus `downvotes`) at the end of voting period  
    /// - `maxActionsPerProposal` - Maximum number of actions (transfers and/or smart contract calls) that a proposal may have  
    /// - `votingDelayInBlocks` - Number of blocks to wait after a block is proposed before being able to vote/downvote that proposal
    /// - `votingPeriodInBlocks` - Number of blocks the voting period lasts (voting delay does not count towards this)  
    /// - `lockTimeAfterVotingEndsInBlocks` - Number of blocks to wait before a successful proposal can be executed  
    #[init]
    fn init(
        &self,
        min_energy_for_propose: BigUint,
        min_fee_for_propose: BigUint,
        quorum_percentage: BigUint,
        voting_delay_in_blocks: u64,
        voting_period_in_blocks: u64,
        withdraw_percentage_defeated: u64,
        energy_factory_address: ManagedAddress,
        fees_collector_address: ManagedAddress,
        fee_token: TokenIdentifier,
    ) {
        self.try_change_min_energy_for_propose(min_energy_for_propose);
        self.try_change_min_fee_for_propose(min_fee_for_propose);
        self.try_change_quorum(quorum_percentage);
        self.try_change_voting_delay_in_blocks(voting_delay_in_blocks);
        self.try_change_voting_period_in_blocks(voting_period_in_blocks);
        self.try_change_withdraw_percentage_defeated(withdraw_percentage_defeated);
        self.set_energy_factory_address(energy_factory_address);
        self.fees_collector_address().set(&fees_collector_address);
        self.try_change_fee_token_id(fee_token);
    }

    /// Propose a list of actions.
    /// A maximum of MAX_GOVERNANCE_PROPOSAL_ACTIONS can be proposed at a time.
    ///
    /// An action has the following format:
    ///     - gas limit for action execution
    ///     - destination address
    ///     - a fee payment for proposal (if smaller than min_fee_for_propose, state: WaitForFee)
    ///     - endpoint to be called on the destination
    ///     - a vector of arguments for the endpoint, in the form of ManagedVec<ManagedBuffer>
    ///
    /// The proposer's energy is NOT automatically used for voting. A separate vote is needed.
    ///
    /// Returns the ID of the newly created proposal.
    #[payable("*")]
    #[endpoint]
    fn propose(
        &self,
        description: ManagedBuffer,
        actions: MultiValueEncoded<GovernanceActionAsMultiArg<Self::Api>>,
    ) -> ProposalId {
        self.require_caller_not_self();
        require!(!actions.is_empty(), "Proposal has no actions");
        require!(
            actions.len() <= MAX_GOVERNANCE_PROPOSAL_ACTIONS,
            "Exceeded max actions per proposal"
        );

        let proposer = self.blockchain().get_caller();
        let user_energy = self.get_energy_amount_non_zero(&proposer);
        let min_energy_for_propose = self.min_energy_for_propose().get();
        require!(user_energy >= min_energy_for_propose, NOT_ENOUGH_ENERGY);

        let user_fee = self.call_value().single_esdt();
        require!(
            self.fee_token_id().get() == user_fee.token_identifier,
            WRONG_TOKEN_ID
        );
        require!(
            self.min_fee_for_propose().get() == user_fee.amount,
            NOT_ENOUGH_FEE
        );

        let mut gov_actions = ArrayVec::new();
        for action_multiarg in actions {
            let gov_action = GovernanceAction::from(action_multiarg);
            require!(
                gov_action.gas_limit < MAX_GAS_LIMIT_PER_BLOCK,
                "A single action cannot use more than the max gas limit per block"
            );

            unsafe {
                gov_actions.push_unchecked(gov_action);
            }
        }

        require!(
            self.total_gas_needed(&gov_actions) < MAX_GAS_LIMIT_PER_BLOCK,
            TOO_MUCH_GAS
        );

        let minimum_quorum = self.quorum_percentage().get();
        let voting_delay_in_blocks = self.voting_delay_in_blocks().get();
        let voting_period_in_blocks = self.voting_period_in_blocks().get();
        let withdraw_percentage_defeated = self.withdraw_percentage_defeated().get();
        let current_block = self.blockchain().get_block_nonce();

        let proposal = GovernanceProposal {
            proposal_id: self.proposals().len() + 1,
            proposer: proposer.clone(),
            description,
            actions: gov_actions,
            fee_payment: user_fee,
            minimum_quorum,
            voting_delay_in_blocks,
            voting_period_in_blocks,
            withdraw_percentage_defeated,
            total_energy: BigUint::zero(),
            proposal_start_block: current_block,
        };
        let proposal_id = self.proposals().push(&proposal);

        self.proposal_votes(proposal_id)
            .set(ProposalVotes::default());
        self.proposal_created_event(proposal_id, &proposer, current_block, &proposal);

        proposal_id
    }

    /// Vote on a proposal. The voting power depends on the user's energy.
    #[endpoint]
    fn vote(&self, proposal_id: ProposalId, vote: VoteType) {
        self.require_caller_not_self();
        self.require_valid_proposal_id(proposal_id);
        require!(
            self.get_proposal_status(proposal_id) == GovernanceProposalStatus::Active,
            PROPOSAL_NOT_ACTIVE
        );

        let voter = self.blockchain().get_caller();
        let new_user = self.user_voted_proposals(&voter).insert(proposal_id);
        require!(new_user, ALREADY_VOTED_ERR_MSG);

        let current_quorum = self.proposal_votes(proposal_id).get().quorum;

        // First voter -> update total_energy
        if current_quorum == BigUint::zero() {
            let fees_collector_addr = self.fees_collector_address().get();
            let last_global_update_week: Week = self
                .fees_collector_proxy(fees_collector_addr.clone())
                .last_global_update_week()
                .execute_on_dest_context();

            let total_energy: BigUint = self
                .fees_collector_proxy(fees_collector_addr)
                .total_energy_for_week(last_global_update_week)
                .execute_on_dest_context();

            let mut proposal = self.proposals().get(proposal_id);
            proposal.total_energy = total_energy;
            self.proposals().set(proposal_id, &proposal);
        }

        let user_energy = self.get_energy_amount_non_zero(&voter);
        let voting_power = user_energy.sqrt();

        match vote {
            VoteType::UpVote => {
                self.proposal_votes(proposal_id).update(|proposal_votes| {
                    proposal_votes.up_votes += &voting_power.clone();
                    proposal_votes.quorum += &user_energy.clone();
                });
                self.up_vote_cast_event(&voter, proposal_id, &voting_power);
            }
            VoteType::DownVote => {
                self.proposal_votes(proposal_id).update(|proposal_votes| {
                    proposal_votes.down_votes += &voting_power.clone();
                    proposal_votes.quorum += &user_energy.clone();
                });
                self.down_vote_cast_event(&voter, proposal_id, &voting_power);
            }
            VoteType::DownVetoVote => {
                self.proposal_votes(proposal_id).update(|proposal_votes| {
                    proposal_votes.down_veto_votes += &voting_power.clone();
                    proposal_votes.quorum += &user_energy.clone();
                });
                self.down_veto_vote_cast_event(&voter, proposal_id, &voting_power);
            }
            VoteType::AbstainVote => {
                self.proposal_votes(proposal_id).update(|proposal_votes| {
                    proposal_votes.abstain_votes += &voting_power.clone();
                    proposal_votes.quorum += &user_energy.clone();
                });
                self.abstain_vote_cast_event(&voter, proposal_id, &voting_power);
            }
        }
    }

    /// Cancel a proposed action. This can be done:
    /// - by the proposer, at any time
    /// - by anyone, if the proposal was defeated
    #[endpoint]
    fn cancel(&self, proposal_id: ProposalId) {
        self.require_caller_not_self();

        match self.get_proposal_status(proposal_id) {
            GovernanceProposalStatus::None => {
                sc_panic!("Proposal does not exist");
            }
            GovernanceProposalStatus::Pending => {
                let proposal = self.proposals().get(proposal_id);
                let caller = self.blockchain().get_caller();

                require!(
                    caller == proposal.proposer,
                    "Only original proposer may cancel a pending proposal"
                );
                self.refund_proposal_fee(proposal_id, &proposal.fee_payment.amount);
                self.clear_proposal(proposal_id);
                self.proposal_canceled_event(proposal_id);
            }
            _ => {
                sc_panic!("Action may not be cancelled");
            }
        }
    }

    /// When a proposal was defeated, the proposer can withdraw
    /// a part of the FEE.
    #[endpoint(withdrawDeposit)]
    fn withdraw_deposit(&self, proposal_id: ProposalId) {
        self.require_caller_not_self();
        let caller = self.blockchain().get_caller();

        match self.get_proposal_status(proposal_id) {
            GovernanceProposalStatus::None => {
                sc_panic!("Proposal does not exist");
            }
            GovernanceProposalStatus::Succeeded | GovernanceProposalStatus::Defeated => {
                let proposal = self.proposals().get(proposal_id);

                require!(
                    caller == proposal.proposer,
                    "Only original proposer may cancel a pending proposal"
                );

                self.refund_proposal_fee(proposal_id, &proposal.fee_payment.amount);
            }
            GovernanceProposalStatus::DefeatedWithVeto => {
                let proposal = self.proposals().get(proposal_id);
                let refund_percentage = BigUint::from(proposal.withdraw_percentage_defeated);
                let refund_amount =
                    refund_percentage * proposal.fee_payment.amount.clone() / FULL_PERCENTAGE;

                require!(
                    caller == proposal.proposer,
                    "Only original proposer may cancel a pending proposal"
                );

                self.refund_proposal_fee(proposal_id, &refund_amount);
                let remaining_fee = proposal.fee_payment.amount - refund_amount;

                self.proposal_remaining_fees().update(|fees| {
                    fees.push(EsdtTokenPayment::new(
                        proposal.fee_payment.token_identifier,
                        proposal.fee_payment.token_nonce,
                        remaining_fee,
                    ));
                });
            }
            _ => {
                sc_panic!("You may not withdraw funds from this proposal!");
            }
        }
        self.proposal_withdraw_after_defeated_event(proposal_id);
    }

    fn total_gas_needed(
        &self,
        actions: &ArrayVec<GovernanceAction<Self::Api>, MAX_GOVERNANCE_PROPOSAL_ACTIONS>,
    ) -> u64 {
        let mut total = 0;
        for action in actions {
            total += action.gas_limit;
        }

        total
    }

    fn refund_proposal_fee(&self, proposal_id: ProposalId, refund_amount: &BigUint) {
        let proposal: GovernanceProposal<<Self as ContractBase>::Api> =
            self.proposals().get(proposal_id);

        self.send().direct_esdt(
            &proposal.proposer,
            &proposal.fee_payment.token_identifier,
            proposal.fee_payment.token_nonce,
            refund_amount,
        );
    }

    #[storage_mapper("proposalRemainingFees")]
    fn proposal_remaining_fees(&self)
        -> SingleValueMapper<ManagedVec<EsdtTokenPayment<Self::Api>>>;
}
