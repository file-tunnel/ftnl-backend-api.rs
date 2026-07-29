// Finite executable model of File Tunnel capability exchange, one-time event
// tickets, file lifecycle ordering, completion, cancellation, and expiry.
module file_tunnel_protocol {
  type Phase = Waiting | Connected | Transferring | Complete | Cancelled | Expired

  pure val FILES = Set(1, 2)
  pure val TICKETS = Set(1, 2)

  type State = {
    phase: Phase,
    pairingAvailable: bool,
    phoneCapabilityIssued: bool,
    issuedTickets: Set[int],
    activeTickets: Set[int],
    declaredFiles: Set[int],
    availableFiles: Set[int],
    downloadedFiles: Set[int],
    sawClaim: bool,
    sawTicketRedemption: bool,
    sawCompletion: bool,
    sawTerminalPhase: bool,
  }

  var s: State

  action init = {
    s' = {
      phase: Waiting,
      pairingAvailable: true,
      phoneCapabilityIssued: false,
      issuedTickets: Set(),
      activeTickets: Set(),
      declaredFiles: Set(),
      availableFiles: Set(),
      downloadedFiles: Set(),
      sawClaim: false,
      sawTicketRedemption: false,
      sawCompletion: false,
      sawTerminalPhase: false,
    }
  }

  action claim = all {
    s.phase == Waiting,
    s.pairingAvailable,
    not(s.phoneCapabilityIssued),
    s' = {
      ...s,
      phase: Connected,
      pairingAvailable: false,
      phoneCapabilityIssued: true,
      sawClaim: true,
    },
  }

  // Desktop can mint before pairing; phone can mint only after capability
  // exchange. Ticket identities are never reissued.
  action mint_desktop_ticket(ticket) = all {
    s.phase != Complete,
    s.phase != Cancelled,
    s.phase != Expired,
    TICKETS.contains(ticket),
    not(s.issuedTickets.contains(ticket)),
    s' = {
      ...s,
      issuedTickets: s.issuedTickets.union(Set(ticket)),
      activeTickets: s.activeTickets.union(Set(ticket)),
    },
  }

  action mint_phone_ticket(ticket) = all {
    s.phoneCapabilityIssued,
    s.phase != Complete,
    s.phase != Cancelled,
    s.phase != Expired,
    TICKETS.contains(ticket),
    not(s.issuedTickets.contains(ticket)),
    s' = {
      ...s,
      issuedTickets: s.issuedTickets.union(Set(ticket)),
      activeTickets: s.activeTickets.union(Set(ticket)),
    },
  }

  action redeem_ticket(ticket) = all {
    s.phase != Complete,
    s.phase != Cancelled,
    s.phase != Expired,
    s.activeTickets.contains(ticket),
    s' = {
      ...s,
      activeTickets: s.activeTickets.exclude(Set(ticket)),
      sawTicketRedemption: true,
    },
  }

  action declare_file(file) = all {
    or { s.phase == Connected, s.phase == Transferring },
    s.phoneCapabilityIssued,
    FILES.contains(file),
    not(s.declaredFiles.contains(file)),
    s' = { ...s, declaredFiles: s.declaredFiles.union(Set(file)) },
  }

  action finish_upload(file) = all {
    or { s.phase == Connected, s.phase == Transferring },
    s.phoneCapabilityIssued,
    s.declaredFiles.contains(file),
    not(s.availableFiles.contains(file)),
    s' = {
      ...s,
      phase: Transferring,
      availableFiles: s.availableFiles.union(Set(file)),
    },
  }

  action download_partial(file) = all {
    or { s.phase == Connected, s.phase == Transferring },
    s.availableFiles.contains(file),
    not(s.downloadedFiles.contains(file)),
    s.downloadedFiles.union(Set(file)) != s.declaredFiles,
    s' = {
      ...s,
      phase: Transferring,
      downloadedFiles: s.downloadedFiles.union(Set(file)),
    },
  }

  action download_last(file) = all {
    or { s.phase == Connected, s.phase == Transferring },
    s.availableFiles.contains(file),
    not(s.downloadedFiles.contains(file)),
    s.downloadedFiles.union(Set(file)) == s.declaredFiles,
    s' = {
      ...s,
      phase: Complete,
      downloadedFiles: s.downloadedFiles.union(Set(file)),
      activeTickets: Set(),
      sawCompletion: true,
      sawTerminalPhase: true,
    },
  }

  action cancel = all {
    or { s.phase == Waiting, s.phase == Connected, s.phase == Transferring },
    s' = {
      ...s,
      phase: Cancelled,
      activeTickets: Set(),
      sawTerminalPhase: true,
    },
  }

  action expire = all {
    or { s.phase == Waiting, s.phase == Connected, s.phase == Transferring },
    s' = {
      ...s,
      phase: Expired,
      activeTickets: Set(),
      sawTerminalPhase: true,
    },
  }

  action idle = s' = s

  action step = {
    nondet file = FILES.oneOf()
    nondet ticket = TICKETS.oneOf()
    any {
      claim,
      mint_desktop_ticket(ticket),
      mint_phone_ticket(ticket),
      redeem_ticket(ticket),
      declare_file(file),
      finish_upload(file),
      download_partial(file),
      download_last(file),
      cancel,
      expire,
      idle,
    }
  }

  val capability_exchange_is_atomic =
    s.pairingAvailable != s.phoneCapabilityIssued

  val one_time_tickets = and {
    s.activeTickets.subseteq(s.issuedTickets),
    s.issuedTickets.subseteq(TICKETS),
  }

  val file_lifecycle_is_ordered = and {
    s.declaredFiles.subseteq(FILES),
    s.availableFiles.subseteq(s.declaredFiles),
    s.downloadedFiles.subseteq(s.availableFiles),
  }

  val phase_is_consistent = and {
    s.phase == Waiting implies not(s.phoneCapabilityIssued),
    or { s.phase == Connected, s.phase == Transferring, s.phase == Complete }
      implies s.phoneCapabilityIssued,
    s.phase == Complete implies and {
      s.declaredFiles != Set(),
      s.downloadedFiles == s.declaredFiles,
    },
    or { s.phase == Complete, s.phase == Cancelled, s.phase == Expired }
      implies s.activeTickets == Set(),
  }

  val protocol_safety = and {
    capability_exchange_is_atomic,
    one_time_tickets,
    file_lifecycle_is_ordered,
    phase_is_consistent,
  }

  val claim_reached = s.sawClaim
  val ticket_redeemed_reached = s.sawTicketRedemption
  val transfer_complete_reached = s.sawCompletion
  val terminal_phase_reached = s.sawTerminalPhase
}
