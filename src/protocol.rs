//! Pure File Tunnel capability and lifecycle policy.
//!
//! HTTP handlers use [`permits`] as their authorization matrix. The compact
//! [`ProtocolState`] below is also exercised by property tests and mirrors the
//! finite Quint model in `formal/file_tunnel_protocol.qnt`.

use crate::TunnelStatus;

const MODEL_FILE_COUNT: u8 = 2;
const MODEL_TICKET_COUNT: u8 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Principal {
    PairingSecret,
    Desktop,
    Phone,
    EventTicket,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Claim,
    ReadSnapshot,
    Cancel,
    DeclareFile,
    UploadFile,
    DownloadFile,
    MintEventTicket,
    RedeemEventTicket,
}

/// Returns whether a capability scope may attempt an operation in this phase.
///
/// Possession and expiry checks remain the responsibility of the caller.
#[must_use]
pub const fn permits(status: TunnelStatus, principal: Principal, operation: Operation) -> bool {
    if matches!(status, TunnelStatus::Cancelled | TunnelStatus::Expired) {
        return false;
    }

    match operation {
        Operation::Claim => {
            matches!(status, TunnelStatus::Waiting) && matches!(principal, Principal::PairingSecret)
        }
        Operation::ReadSnapshot => {
            matches!(principal, Principal::Desktop | Principal::Phone)
        }
        Operation::Cancel => {
            !matches!(status, TunnelStatus::Complete) && matches!(principal, Principal::Desktop)
        }
        Operation::DeclareFile | Operation::UploadFile => {
            matches!(status, TunnelStatus::Connected | TunnelStatus::Transferring)
                && matches!(principal, Principal::Phone)
        }
        Operation::DownloadFile => {
            matches!(status, TunnelStatus::Connected | TunnelStatus::Transferring)
                && matches!(principal, Principal::Desktop)
        }
        Operation::MintEventTicket => {
            !matches!(status, TunnelStatus::Complete)
                && matches!(principal, Principal::Desktop | Principal::Phone)
        }
        Operation::RedeemEventTicket => {
            !matches!(status, TunnelStatus::Complete) && matches!(principal, Principal::EventTicket)
        }
    }
}

#[must_use]
pub const fn progress_is_valid(size_bytes: u64, bytes_transferred: u64) -> bool {
    bytes_transferred <= size_bytes
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtocolAction {
    Claim { principal: Principal },
    MintEventTicket { principal: Principal, ticket: u8 },
    RedeemEventTicket { principal: Principal, ticket: u8 },
    DeclareFile { principal: Principal, file: u8 },
    FinishUpload { principal: Principal, file: u8 },
    DownloadFile { principal: Principal, file: u8 },
    Cancel { principal: Principal },
    Expire,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionError {
    Unauthorized,
    Conflict,
    OutsideFiniteDomain,
}

/// A deliberately small executable refinement of the protocol model.
///
/// Files and event tickets are represented as two-bit sets. This is not runtime
/// storage; it keeps randomized and bounded verification fast while preserving
/// the capability and lifecycle relationships that matter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProtocolState {
    pub status: TunnelStatus,
    pub pairing_available: bool,
    pub phone_capability_issued: bool,
    pub issued_tickets: u8,
    pub active_tickets: u8,
    pub declared_files: u8,
    pub available_files: u8,
    pub downloaded_files: u8,
}

impl Default for ProtocolState {
    fn default() -> Self {
        Self::initial()
    }
}

impl ProtocolState {
    #[must_use]
    pub const fn initial() -> Self {
        Self {
            status: TunnelStatus::Waiting,
            pairing_available: true,
            phone_capability_issued: false,
            issued_tickets: 0,
            active_tickets: 0,
            declared_files: 0,
            available_files: 0,
            downloaded_files: 0,
        }
    }

    #[must_use]
    pub const fn invariants_hold(self) -> bool {
        let domain_mask = (1_u8 << MODEL_FILE_COUNT) - 1;
        let ticket_mask = (1_u8 << MODEL_TICKET_COUNT) - 1;
        let sets_are_bounded = self.declared_files & !domain_mask == 0
            && self.available_files & !domain_mask == 0
            && self.downloaded_files & !domain_mask == 0
            && self.issued_tickets & !ticket_mask == 0
            && self.active_tickets & !ticket_mask == 0;
        let file_ordering = self.available_files & !self.declared_files == 0
            && self.downloaded_files & !self.available_files == 0;
        let ticket_ordering = self.active_tickets & !self.issued_tickets == 0;
        let capability_exchange = self.pairing_available != self.phone_capability_issued;
        let phase_consistency = match self.status {
            TunnelStatus::Waiting => !self.phone_capability_issued,
            TunnelStatus::Connected | TunnelStatus::Transferring => self.phone_capability_issued,
            TunnelStatus::Complete => {
                self.phone_capability_issued
                    && self.declared_files != 0
                    && self.downloaded_files == self.declared_files
                    && self.active_tickets == 0
            }
            TunnelStatus::Cancelled | TunnelStatus::Expired => true,
        };

        sets_are_bounded
            && file_ordering
            && ticket_ordering
            && capability_exchange
            && phase_consistency
    }

    pub fn apply(&mut self, action: ProtocolAction) -> Result<(), TransitionError> {
        let before = *self;
        let result = self.apply_inner(action);
        if result.is_err() {
            *self = before;
        }
        debug_assert!(self.invariants_hold());
        result
    }

    fn apply_inner(&mut self, action: ProtocolAction) -> Result<(), TransitionError> {
        match action {
            ProtocolAction::Claim { principal } => {
                self.authorize(principal, Operation::Claim)?;
                if !self.pairing_available || self.phone_capability_issued {
                    return Err(TransitionError::Conflict);
                }
                self.pairing_available = false;
                self.phone_capability_issued = true;
                self.status = TunnelStatus::Connected;
            }
            ProtocolAction::MintEventTicket { principal, ticket } => {
                self.authorize(principal, Operation::MintEventTicket)?;
                let bit = domain_bit(ticket, MODEL_TICKET_COUNT)?;
                if self.issued_tickets & bit != 0 {
                    return Err(TransitionError::Conflict);
                }
                self.issued_tickets |= bit;
                self.active_tickets |= bit;
            }
            ProtocolAction::RedeemEventTicket { principal, ticket } => {
                self.authorize(principal, Operation::RedeemEventTicket)?;
                let bit = domain_bit(ticket, MODEL_TICKET_COUNT)?;
                if self.active_tickets & bit == 0 {
                    return Err(TransitionError::Conflict);
                }
                self.active_tickets &= !bit;
            }
            ProtocolAction::DeclareFile { principal, file } => {
                self.authorize(principal, Operation::DeclareFile)?;
                let bit = domain_bit(file, MODEL_FILE_COUNT)?;
                if self.declared_files & bit != 0 {
                    return Err(TransitionError::Conflict);
                }
                self.declared_files |= bit;
            }
            ProtocolAction::FinishUpload { principal, file } => {
                self.authorize(principal, Operation::UploadFile)?;
                let bit = domain_bit(file, MODEL_FILE_COUNT)?;
                if self.declared_files & bit == 0 || self.available_files & bit != 0 {
                    return Err(TransitionError::Conflict);
                }
                self.available_files |= bit;
                self.status = TunnelStatus::Transferring;
            }
            ProtocolAction::DownloadFile { principal, file } => {
                self.authorize(principal, Operation::DownloadFile)?;
                let bit = domain_bit(file, MODEL_FILE_COUNT)?;
                if self.available_files & bit == 0 || self.downloaded_files & bit != 0 {
                    return Err(TransitionError::Conflict);
                }
                self.downloaded_files |= bit;
                if self.downloaded_files == self.declared_files {
                    self.status = TunnelStatus::Complete;
                    self.active_tickets = 0;
                }
            }
            ProtocolAction::Cancel { principal } => {
                self.authorize(principal, Operation::Cancel)?;
                self.status = TunnelStatus::Cancelled;
                self.active_tickets = 0;
            }
            ProtocolAction::Expire => {
                if matches!(
                    self.status,
                    TunnelStatus::Complete | TunnelStatus::Cancelled | TunnelStatus::Expired
                ) {
                    return Err(TransitionError::Conflict);
                }
                self.status = TunnelStatus::Expired;
                self.active_tickets = 0;
            }
        }
        Ok(())
    }

    fn authorize(self, principal: Principal, operation: Operation) -> Result<(), TransitionError> {
        let capability_exists = match principal {
            Principal::PairingSecret => self.pairing_available,
            Principal::Desktop => true,
            Principal::Phone => self.phone_capability_issued,
            Principal::EventTicket => true,
        };
        if capability_exists && permits(self.status, principal, operation) {
            Ok(())
        } else {
            Err(TransitionError::Unauthorized)
        }
    }
}

fn domain_bit(index: u8, count: u8) -> Result<u8, TransitionError> {
    if index >= count {
        return Err(TransitionError::OutsideFiniteDomain);
    }
    Ok(1_u8 << index)
}

#[cfg(kani)]
mod verification {
    use super::*;

    #[kani::proof]
    fn capability_scopes_do_not_cross_privilege_boundaries() {
        let status = match kani::any::<u8>() % 6 {
            0 => TunnelStatus::Waiting,
            1 => TunnelStatus::Connected,
            2 => TunnelStatus::Transferring,
            3 => TunnelStatus::Complete,
            4 => TunnelStatus::Cancelled,
            _ => TunnelStatus::Expired,
        };

        assert!(!permits(status, Principal::Phone, Operation::Cancel));
        assert!(!permits(status, Principal::Phone, Operation::DownloadFile));
        assert!(!permits(status, Principal::Desktop, Operation::DeclareFile));
        assert!(!permits(status, Principal::Desktop, Operation::UploadFile));
        assert!(!permits(
            status,
            Principal::PairingSecret,
            Operation::ReadSnapshot
        ));
    }

    #[kani::proof]
    fn progress_validation_is_the_declared_bound() {
        let size = kani::any::<u64>();
        let transferred = kani::any::<u64>();
        assert_eq!(progress_is_valid(size, transferred), transferred <= size);
    }

    #[kani::proof]
    fn claim_is_an_atomic_one_time_exchange() {
        let mut state = ProtocolState::initial();
        assert!(state
            .apply(ProtocolAction::Claim {
                principal: Principal::PairingSecret,
            })
            .is_ok());
        let claimed = state;
        assert!(state
            .apply(ProtocolAction::Claim {
                principal: Principal::PairingSecret,
            })
            .is_err());
        assert_eq!(state, claimed);
        assert!(!state.pairing_available);
        assert!(state.phone_capability_issued);
        assert!(state.invariants_hold());
    }

    #[kani::proof]
    fn rejected_finite_domain_transition_is_immutable() {
        let mut state = ProtocolState::initial();
        let before = state;
        let identity = kani::any::<u8>();
        kani::assume(identity >= MODEL_TICKET_COUNT);
        assert!(state
            .apply(ProtocolAction::MintEventTicket {
                principal: Principal::Desktop,
                ticket: identity,
            })
            .is_err());
        assert_eq!(state, before);
    }
}
