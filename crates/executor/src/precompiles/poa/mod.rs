pub mod cosig;

use super::pos::{
    Amount, Power, Punishment, Score, StakerIDRef, Staking, ValidatorID, ValidatorIDRef,
    ValidatorW as Validator,
};
use cosig::{CoSigChecker, CoSigOp, CoSigRule, ValidatorPubKey};
use ruc::{crypto::hash, *};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fmt::Debug};
use vsdb_trie_map::{TrieHash, TrieMap};

pub struct Auth {
    // ValidatorID => {struct ValidatorWeight}
    committee: TrieMap,

    // Cosig rule
    rule: CoSigRule,

    // Embed staking moduler
    staking: Staking,

    root: TrieHash,
}

impl Auth {
    pub fn new(
        committee: BTreeMap<ValidatorID, ValidatorWeight>,
        cosig_rule: Option<CoSigRule>,
        score_max: Option<Score>,
        score_min_for_offline: Option<Score>,
        validator_cap: Option<u32>,
        cache_size: Option<usize>,
    ) -> Result<Self> {
        let mut c = TrieMap::create(cache_size).c(d!())?;
        for (id, v) in committee.iter() {
            c.insert(id, &v.to_bytes()).c(d!())?;
        }
        let r = cosig_rule.unwrap_or_default();
        let s =
            Staking::new(score_max, score_min_for_offline, validator_cap, cache_size)
                .c(d!())?;

        let mut me = Self {
            committee: c,
            rule: r,
            staking: s,
            root: Default::default(),
        };

        me.commit();

        Ok(me)
    }

    pub fn validator_cap(&self) -> u32 {
        self.staking.validator_cap()
    }

    pub fn set_validator_cap(&mut self, n: u32) {
        self.staking.set_validator_cap(n);
    }

    pub fn root(&self) -> TrieHash {
        self.root
    }

    /// Call this API after all changes
    pub fn commit(&mut self) -> TrieHash {
        self.root = hash_all(&[
            &self.committee.commit(),
            &hash(&self.rule.to_bytes()),
            &self.staking.commit(),
        ]);
        self.root
    }

    /// Do a full replacement for the current validator set
    pub fn refresh_validators(
        &mut self,
        op: CoSigOp<BTreeMap<ValidatorID, Validator>>,
    ) -> Result<()> {
        self.check_cosigs(&op)
            .c(d!())
            .and_then(|_| self.staking.refresh_validators(&op.data).c(d!()))
    }

    /// Do a full replacement for the current committee
    pub fn refresh_committee(
        &mut self,
        op: CoSigOp<BTreeMap<ValidatorID, ValidatorWeight>>,
    ) -> Result<()> {
        self.check_cosigs(&op).c(d!())?;

        self.committee.clear().c(d!())?;

        for (id, vw) in op.data.iter() {
            self.committee.insert(id, &vw.to_bytes()).c(d!())?;
        }

        Ok(())
    }

    fn check_cosigs<T>(&self, op: &CoSigOp<T>) -> Result<()>
    where
        T: Serialize + for<'a> Deserialize<'a>,
    {
        let mut c = BTreeMap::new();
        for i in self.committee.ro_handle(self.committee.root()).iter() {
            let (_, vw) = i.c(d!())?;
            let vw = ValidatorWeight::from_bytes(&vw).c(d!())?;
            c.insert(vw.pubkey, vw.weight);
        }
        let cc = CoSigChecker {
            rule: self.rule,
            committee: c,
        };

        op.check_cosigs(&cc).c(d!())
    }

    pub fn get_validators(&self) -> Result<Vec<Validator>> {
        self.staking.get_w_validators().c(d!())
    }

    pub fn get_validator(&self, id: ValidatorIDRef) -> Result<Validator> {
        self.staking.get_w_validator(id).c(d!())
    }

    /// NOTE:
    /// this function only executes the logic directly related to staking,
    /// and it is not responsible for checking gas, nonce, blance, etc.
    pub fn stake_to(
        &mut self,
        staker: StakerIDRef,
        validator: ValidatorIDRef,
        amount: Amount,
    ) -> Result<()> {
        self.staking
            .stake_to(staker, validator, amount, true)
            .c(d!())
    }

    /// Returns the total amount successfully unstaked.
    ///
    /// NOTE:
    /// this function only executes the logic directly related to staking,
    /// and it is not responsible for checking gas, nonce, blance, etc.
    pub fn unstake_from(
        &mut self,
        staker: StakerIDRef,
        validator: ValidatorIDRef,
        amount: Option<Amount>,
    ) -> Result<Amount> {
        self.staking.unstake_from(staker, validator, amount).c(d!())
    }

    /// Returns the total amount successfully unstaked.
    ///
    /// NOTE:
    /// this function only executes the logic directly related to staking,
    /// and it is not responsible for checking gas, nonce, blance, etc.
    pub fn unstake_all(&mut self, staker: StakerIDRef) -> Result<Amount> {
        self.staking.unstake_all(staker).c(d!())
    }

    pub fn get_validator_score(&self, id: ValidatorIDRef) -> Result<Score> {
        self.staking.get_validator_score(id).c(d!())
    }

    pub fn get_validator_staking_total(&self, id: ValidatorIDRef) -> Result<Amount> {
        self.staking.get_validator_staking_total(id).c(d!())
    }

    pub fn get_validator_power(&self, id: ValidatorIDRef) -> Result<Amount> {
        self.staking.get_validator_power(id).c(d!())
    }

    pub fn validator_in_formal_list(&self, id: ValidatorIDRef) -> Result<bool> {
        self.staking.validator_in_formal_list(id).c(d!())
    }

    pub fn validator_formal_list(&self) -> Result<BTreeMap<ValidatorID, Power>> {
        self.staking.validator_formal_list().c(d!())
    }

    pub fn governance_with_each_block(
        &mut self,
        governances: Vec<Punishment>,
    ) -> Result<()> {
        self.staking.governance_with_each_block(governances).c(d!())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct ValidatorWeight {
    pubkey: ValidatorPubKey,
    weight: u64,
}

impl ValidatorWeight {
    fn to_bytes(self) -> Vec<u8> {
        pnk!(bcs::to_bytes(&self))
    }

    fn from_bytes(bytes: &[u8]) -> Result<Self> {
        bcs::from_bytes(bytes).c(d!())
    }
}

fn hash_all(bb: &[&[u8]]) -> TrieHash {
    hash(&concat(bb))
}

fn concat(bb: &[&[u8]]) -> Vec<u8> {
    bb.iter().flat_map(|i| i.iter().copied()).collect()
}
