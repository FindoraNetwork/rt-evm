//!
//! # CoSignature
//!
//! Aka Multi-Signature, mainly used to support updating validator set.
//!

pub use ed25519_zebra::VerificationKeyBytes as ValidatorPubKey;

use ed25519_zebra::{Signature, SigningKey, VerificationKey, VerificationKeyBytes};
use ruc::{crypto::hash, *};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fmt::{self, Debug},
};

/// A common structure for data with co-signatures.
#[derive(Clone, Eq, PartialEq, Deserialize, Serialize)]
#[serde(bound = "")]
pub struct CoSigOp<T>
where
    T: Serialize + for<'a> Deserialize<'a>,
{
    pub(crate) data: T,
    pub(crate) cosigs: BTreeMap<VerificationKeyBytes, CoSig>,
}

impl<T> CoSigOp<T>
where
    T: Serialize + for<'a> Deserialize<'a>,
{
    pub fn create(msg: T) -> Self {
        CoSigOp {
            data: msg,
            cosigs: BTreeMap::new(),
        }
    }

    /// Attach a new signature.
    pub fn sign(&mut self, sk: &SigningKey) -> Result<()> {
        bcs::to_bytes(&self.data).c(d!()).map(|msg| {
            let k = VerificationKeyBytes::from(sk);
            let v = CoSig::new(k, sk.sign(&msg));
            self.cosigs.insert(k, v);
        })
    }

    /// Attach some new signatures in a batch mode.
    pub fn batch_sign(&mut self, sks: &[&SigningKey]) -> Result<()> {
        let msg = bcs::to_bytes(&self.data).c(d!())?;
        sks.iter().for_each(|sk| {
            let k = VerificationKeyBytes::from(*sk);
            let v = CoSig::new(k, sk.sign(&msg));
            self.cosigs.insert(k, v);
        });
        Ok(())
    }

    /// Check if a cosig is valid.
    pub fn check_cosigs(&self, cc: &CoSigChecker) -> Result<()> {
        if cc.committee.is_empty() {
            return Ok(());
        }

        self.check_existence(cc)
            .c(d!())
            .and_then(|_| self.check_weight(cc).c(d!()))
            .and_then(|_| {
                let msg = bcs::to_bytes(&self.data).c(d!())?;
                if self.cosigs.values().any(|cs| {
                    VerificationKey::try_from(cs.vk)
                        .map(|vk| vk.verify(&cs.sig, &msg).is_err())
                        .is_err()
                }) {
                    Err(eg!(CoSigErr::SigInvalid))
                } else {
                    Ok(())
                }
            })
    }

    fn check_existence(&self, cc: &CoSigChecker) -> Result<()> {
        if self.cosigs.keys().any(|k| !cc.committee.contains_key(k)) {
            Err(eg!(CoSigErr::KeyUnknown))
        } else {
            Ok(())
        }
    }

    fn check_weight(&self, cc: &CoSigChecker) -> Result<()> {
        let rule_weights = cc.committee.values().map(|v| *v as u128).sum::<u128>();
        let actual_weights = self
            .cosigs
            .values()
            .flat_map(|s| cc.committee.get(&s.vk).map(|v| *v as u128))
            .sum::<u128>();

        let rule = [cc.rule.threshold[0] as u128, cc.rule.threshold[1] as u128];

        if actual_weights.checked_mul(rule[1]).ok_or(eg!())?
            < rule[0].checked_mul(rule_weights).ok_or(eg!())?
        {
            return Err(eg!(CoSigErr::WeightInsufficient));
        }

        Ok(())
    }

    pub fn hash(&self) -> Result<Vec<u8>> {
        bcs::to_bytes(self)
            .c(d!())
            .map(|bytes| hash(&bytes).to_vec())
    }
}

/// The rule for a kind of data.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct CoSigRule {
    /// check rule:
    /// - `[actual weight].sum() / [rule weight].sum() >= threshold%`
    /// - threshold% = `numerator / denominator` = `threshold[0] / threshold[1]`
    ///
    /// which equal to:
    /// - `[actual weight].sum() * threshold[1] >= threshold[0] * [rule weight].sum()`
    /// - convert to `u128` to avoid integer overflow
    pub threshold: [u64; 2],
}

impl CoSigRule {
    pub fn new(threshold: [u64; 2]) -> Result<Self> {
        if threshold[0] > threshold[1] {
            return Err(eg!("invalid threshold"));
        }

        Ok(CoSigRule {
            threshold: [threshold[0], threshold[1]],
        })
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        pnk!(bcs::to_bytes(self))
    }
}

impl Default for CoSigRule {
    fn default() -> Self {
        Self::new([2, 3]).unwrap()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub(crate) struct CoSig {
    vk: VerificationKeyBytes,
    sig: Signature,
}

impl CoSig {
    fn new(vk: VerificationKeyBytes, sig: Signature) -> Self {
        CoSig { vk, sig }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
enum CoSigErr {
    KeyUnknown,
    SigInvalid,
    WeightInsufficient,
}

impl fmt::Display for CoSigErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let msg = match self {
            CoSigErr::KeyUnknown => "Found keys outside of the predefined rule",
            CoSigErr::WeightInsufficient => "Total weight is lower than the threshold",
            CoSigErr::SigInvalid => "Invalid signature",
        };
        write!(f, "{msg}")
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CoSigChecker {
    pub rule: CoSigRule,
    pub committee: BTreeMap<VerificationKeyBytes, u64>,
}

pub fn gen_sk() -> SigningKey {
    SigningKey::new(rand::rngs::OsRng)
}
