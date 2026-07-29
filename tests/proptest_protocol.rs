use ftnl_backend_api::{
    protocol::{Principal, ProtocolAction, ProtocolState},
    TunnelStatus,
};
use proptest::prelude::*;

fn principal(value: u8) -> Principal {
    match value % 4 {
        0 => Principal::PairingSecret,
        1 => Principal::Desktop,
        2 => Principal::Phone,
        _ => Principal::EventTicket,
    }
}

fn action(kind: u8, actor: u8, identity: u8) -> ProtocolAction {
    let principal = principal(actor);
    match kind % 8 {
        0 => ProtocolAction::Claim { principal },
        1 => ProtocolAction::MintEventTicket {
            principal,
            ticket: identity,
        },
        2 => ProtocolAction::RedeemEventTicket {
            principal,
            ticket: identity,
        },
        3 => ProtocolAction::DeclareFile {
            principal,
            file: identity,
        },
        4 => ProtocolAction::FinishUpload {
            principal,
            file: identity,
        },
        5 => ProtocolAction::DownloadFile {
            principal,
            file: identity,
        },
        6 => ProtocolAction::Cancel { principal },
        _ => ProtocolAction::Expire,
    }
}

proptest! {
    #[test]
    fn arbitrary_traces_preserve_protocol_invariants(
        trace in prop::collection::vec((any::<u8>(), any::<u8>(), 0_u8..4), 0..96)
    ) {
        let mut state = ProtocolState::initial();
        prop_assert!(state.invariants_hold());

        for (kind, actor, identity) in trace {
            let before = state;
            let result = state.apply(action(kind, actor, identity));
            prop_assert!(state.invariants_hold());
            if result.is_err() {
                prop_assert_eq!(state, before);
            }
        }
    }

    #[test]
    fn only_the_pairing_secret_can_claim(actor in any::<u8>()) {
        let mut state = ProtocolState::initial();
        let principal = principal(actor);
        let result = state.apply(ProtocolAction::Claim { principal });

        if principal == Principal::PairingSecret {
            prop_assert!(result.is_ok());
            prop_assert_eq!(state.status, TunnelStatus::Connected);
            prop_assert!(!state.pairing_available);
            prop_assert!(state.phone_capability_issued);
        } else {
            prop_assert!(result.is_err());
            prop_assert_eq!(state, ProtocolState::initial());
        }
    }
}

#[test]
fn pairing_and_event_tickets_are_one_time_capabilities() {
    let mut state = ProtocolState::initial();
    state
        .apply(ProtocolAction::Claim {
            principal: Principal::PairingSecret,
        })
        .unwrap();
    assert!(state
        .apply(ProtocolAction::Claim {
            principal: Principal::PairingSecret,
        })
        .is_err());

    state
        .apply(ProtocolAction::MintEventTicket {
            principal: Principal::Desktop,
            ticket: 0,
        })
        .unwrap();
    state
        .apply(ProtocolAction::RedeemEventTicket {
            principal: Principal::EventTicket,
            ticket: 0,
        })
        .unwrap();
    assert!(state
        .apply(ProtocolAction::RedeemEventTicket {
            principal: Principal::EventTicket,
            ticket: 0,
        })
        .is_err());
}
