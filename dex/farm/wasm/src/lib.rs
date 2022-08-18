////////////////////////////////////////////////////
////////////////// AUTO-GENERATED //////////////////
////////////////////////////////////////////////////

#![no_std]

elrond_wasm_node::wasm_endpoints! {
    farm
    (
        callBack
        addAdmins
        addToPauseWhitelist
        calculateRewardsForGivenPosition
        end_produce_rewards
        enterFarm
        exitFarm
        getAdmins
        getBurnGasLimit
        getDivisionSafetyConstant
        getFarmMigrationConfiguration
        getFarmTokenId
        getFarmTokenSupply
        getFarmingTokenId
        getLastRewardBlockNonce
        getLockedAssetFactoryManagedAddress
        getMinimumFarmingEpoch
        getPairContractManagedAddress
        getPenaltyPercent
        getPerBlockRewardAmount
        getRewardPerShare
        getRewardReserve
        getRewardTokenId
        getState
        mergeFarmTokens
        migrateFromV1_2Farm
        pause
        registerFarmToken
        removeAdmins
        removeFromPauseWhitelist
        resume
        setFarmMigrationConfig
        setFarmTokenSupply
        setPerBlockRewardAmount
        setRpsAndStartRewards
        set_burn_gas_limit
        set_minimum_farming_epochs
        set_penalty_percent
        startProduceRewards
    )
}
