use beserial::{Deserialize, Serialize};
use std::fmt::Display;

#[derive(Serialize, Deserialize, Copy, Clone, Ord, PartialOrd, Eq, PartialEq, Debug, Hash, Display)]
#[repr(u8)]
pub enum NetworkId {
    Test = 1,
    Dev = 2,
    Bounty = 3,
    Dummy = 4,
    Main = 42,
}