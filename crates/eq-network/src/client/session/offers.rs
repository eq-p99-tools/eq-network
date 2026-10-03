//! What a merchant pays for an item sold to them. A front end works it out
//! from the item's price and the rate the merchant opened with, by the
//! server type's rule, so the session sends and hears nothing for it;
//! offering it is the server type's choice, made where the rule has been
//! checked against what the purse gains.
use super::feature::Feature;

/// Lets a front end show what a merchant pays for an item sold to them.
pub(super) struct MerchantOffers;

impl Feature for MerchantOffers {
    fn capabilities(&self) -> Vec<crate::world::Capability> {
        vec![crate::world::Capability::MerchantOffers]
    }
}
