//! Each `SZ_*` static has the size of one type on the target; `types.sh` reads the sizes with llvm-nm.
#![no_std]
use core::mem::size_of;
use zmidi_ci::{CiDevice, DeviceConfig, Event, Muid, Peer, ProfileId, ProfileTable, Received, RequestTable};

macro_rules! sz {
    ($n:ident, $t:ty) => {
        #[unsafe(no_mangle)]
        pub static $n: [u8; size_of::<$t>()] = [0; size_of::<$t>()];
    };
}

sz!(SZ_CiDevice_2_1_4, CiDevice<'static, 2, 1, 4>);
sz!(SZ_CiDevice_1_1_1, CiDevice<'static, 1, 1, 1>);
sz!(SZ_DeviceConfig, DeviceConfig<'static>);
sz!(SZ_Peer, Peer);
sz!(SZ_Peers_2, [Option<Peer>; 2]);
sz!(SZ_DetailsPending_4, [Option<(Muid, ProfileId, u8)>; 4]);
sz!(SZ_RequestTable_4, RequestTable<4>);
sz!(SZ_ProfileTable_1, ProfileTable<1>);
sz!(SZ_Event, Event<'static>);
sz!(SZ_Received_1, Received<'static, 'static, 1>);
sz!(SZ_SysEx7Reassembler_512, zmidi_ump::SysEx7Reassembler<512>);
