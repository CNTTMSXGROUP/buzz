/// Performs the ordered state transition when the configured push gateway changes.
public enum BuzzPushGatewayStateReset {
  /// Journals every retired-gateway record before replacing active state.
  public static func run(
    gatewayOrigin: String,
    records: [BuzzPushEndpointGrantRecord],
    pendingEnrollments: [BuzzPushPendingEnrollmentRecord],
    cleanupStates: [BuzzPushGatewayCleanupState],
    saveCleanupState: (BuzzPushGatewayCleanupState) throws -> Void,
    replaceRecords: ([BuzzPushEndpointGrantRecord]) throws -> Void,
    replacePendingEnrollments: ([BuzzPushPendingEnrollmentRecord]) throws -> Void
  ) throws {
    let staleRecords = records.filter { $0.gatewayOrigin != gatewayOrigin }
    let stalePending = pendingEnrollments.filter { $0.gatewayOrigin != gatewayOrigin }
    let staleOrigins = Set(staleRecords.map(\.gatewayOrigin) + stalePending.map(\.gatewayOrigin))

    for origin in staleOrigins.sorted() {
      var state = cleanupStates.first { $0.gatewayOrigin == origin }
        ?? BuzzPushGatewayCleanupState(
          gatewayOrigin: origin,
          grants: [],
          pendingEnrollments: []
        )
      for record in staleRecords where record.gatewayOrigin == origin {
        state.grants.removeAll {
          $0.relayOrigin == record.relayOrigin && $0.appProfile == record.appProfile
        }
        state.grants.append(record)
      }
      for pending in stalePending where pending.gatewayOrigin == origin {
        state.pendingEnrollments.removeAll {
          $0.relayOrigin == pending.relayOrigin && $0.appProfile == pending.appProfile
        }
        state.pendingEnrollments.append(pending)
      }
      try saveCleanupState(state)
    }

    if !staleRecords.isEmpty {
      try replaceRecords(records.filter { $0.gatewayOrigin == gatewayOrigin })
    }
    if !stalePending.isEmpty {
      try replacePendingEnrollments(
        pendingEnrollments.filter { $0.gatewayOrigin == gatewayOrigin }
      )
    }
  }
}
