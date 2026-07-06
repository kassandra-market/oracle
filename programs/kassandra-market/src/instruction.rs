//! Instruction wire format. First byte of `instruction_data` = discriminant.
//! Discriminants are a stable public contract; append, never renumber.

#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ix {
    InitConfig = 0,
    UpdateConfig = 1,
    CreateMarket = 2,
    Contribute = 3,
    Cancel = 4,
    Refund = 5,
    Activate = 6,
    ClaimLp = 7,
    ResolveMarket = 8,
    CollectFee = 9,
    CloseMarket = 10,
}

impl Ix {
    pub fn from_u8(x: u8) -> Option<Self> {
        match x {
            0 => Some(Ix::InitConfig),
            1 => Some(Ix::UpdateConfig),
            2 => Some(Ix::CreateMarket),
            3 => Some(Ix::Contribute),
            4 => Some(Ix::Cancel),
            5 => Some(Ix::Refund),
            6 => Some(Ix::Activate),
            7 => Some(Ix::ClaimLp),
            8 => Some(Ix::ResolveMarket),
            9 => Some(Ix::CollectFee),
            10 => Some(Ix::CloseMarket),
            _ => None,
        }
    }
}
