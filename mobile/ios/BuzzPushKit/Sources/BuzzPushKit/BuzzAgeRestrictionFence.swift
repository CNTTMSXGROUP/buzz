import Foundation

/// Cross-process generation fence for age-restricted notification delivery.
public struct BuzzAgeRestrictionFence: Codable, Equatable, Sendable {
  /// Opaque generation changed at both ends of a restricted cleanup.
  public let token: String

  /// Whether native notification state is still being cleared.
  public let isFencing: Bool

  /// Creates a persisted cross-process fence value.
  public init(token: String, isFencing: Bool) {
    self.token = token
    self.isFencing = isFencing
  }

  /// Default value before any restricted cleanup has begun.
  public static let initial = BuzzAgeRestrictionFence(
    token: "initial",
    isFencing: false
  )

  /// Fail-closed value used when the shared fence cannot be read.
  public static let unavailable = BuzzAgeRestrictionFence(
    token: "unavailable",
    isFencing: true
  )

  /// Whether an extension started under [earlier] must discard its result.
  public func requiresDiscard(since earlier: BuzzAgeRestrictionFence) -> Bool {
    isFencing || token != earlier.token
  }
}

/// The app begins a durable fence before clearing native notification state.
/// A notification extension discards resolved content while the fence is
/// active or whenever the token differs from the one captured at startup.
public final class BuzzAgeRestrictionFenceStore: @unchecked Sendable {
  /// App-group file shared by Runner and the notification extension.
  public static let fileName = "age-restriction-fence.json"

  private let fileURL: URL
  private let lock = NSLock()

  /// Creates a fence store rooted in the app-group container.
  public init(containerURL: URL) {
    fileURL = containerURL.appendingPathComponent(Self.fileName)
  }

  /// Returns the latest fence, failing closed for malformed persisted data.
  public func current() -> BuzzAgeRestrictionFence {
    lock.lock()
    defer { lock.unlock() }
    return loadLocked()
  }

  /// Starts a durable cleanup phase with a fresh generation token.
  @discardableResult
  public func begin() throws -> BuzzAgeRestrictionFence {
    lock.lock()
    defer { lock.unlock() }
    let fence = BuzzAgeRestrictionFence(
      token: UUID().uuidString.lowercased(),
      isFencing: true
    )
    try writeLocked(fence)
    return fence
  }

  /// Rotates the durable fence before cleanup begins and settles it only after
  /// every cleanup write succeeds. A thrown cleanup leaves the fence active so
  /// notification extensions continue to fail closed.
  public func performFencedCleanup(_ cleanup: () throws -> Void) throws {
    let active = try begin()
    try cleanup()
    try settleIfFencing(expectedToken: active.token)
  }

  /// Rotates the durable fence before asynchronous cleanup begins and settles
  /// it only after the cleanup callback acknowledges success. A thrown setup
  /// error or callback error leaves the fence active.
  public func performFencedAsyncCleanup(
    _ cleanup: (@escaping (Error?) -> Void) throws -> Void,
    completion: @escaping (Error?) -> Void
  ) throws {
    let active = try begin()
    try cleanup { [self] error in
      guard error == nil else {
        completion(error)
        return
      }
      do {
        try settleIfFencing(expectedToken: active.token)
        completion(nil)
      } catch {
        completion(error)
      }
    }
  }

  /// Completes a cleanup phase with another generation change.
  @discardableResult
  public func settleIfFencing(expectedToken: String? = nil) throws -> BuzzAgeRestrictionFence {
    lock.lock()
    defer { lock.unlock() }
    let current = loadLocked()
    guard current.isFencing,
      expectedToken == nil || current.token == expectedToken
    else { return current }
    let settled = BuzzAgeRestrictionFence(
      token: UUID().uuidString.lowercased(),
      isFencing: false
    )
    try writeLocked(settled)
    return settled
  }

  private func loadLocked() -> BuzzAgeRestrictionFence {
    guard FileManager.default.fileExists(atPath: fileURL.path) else {
      return .initial
    }
    guard let data = try? Data(contentsOf: fileURL),
      let fence = try? JSONDecoder().decode(BuzzAgeRestrictionFence.self, from: data)
    else {
      return .unavailable
    }
    return fence
  }

  private func writeLocked(_ fence: BuzzAgeRestrictionFence) throws {
    let data = try JSONEncoder().encode(fence)
    try data.write(to: fileURL, options: .atomic)
  }
}
