// Business policy after Apple signature verification. No caller-supplied decoded JWT is trusted.
import { createHash } from 'node:crypto';

const uuid = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
function integer(value) { if (!Number.isSafeInteger(value) || value < 0) throw new Error('invalid_apple_event'); return value; }
function identifier(value) { if (typeof value !== 'string' || !/^[0-9]{1,40}$/.test(value)) throw new Error('invalid_apple_event'); return value; }

export function normalizeTransaction(transaction, config, { renewal = null, notificationDate = null, eventID = null, status = null } = {}) {
  if (transaction.bundleId !== config.bundleID || transaction.environment !== config.environment ||
      transaction.productId !== config.productID || transaction.type !== 'Auto-Renewable Subscription' ||
      transaction.inAppOwnershipType !== 'PURCHASED' || !uuid.test(transaction.appAccountToken || '')) throw new Error('invalid_apple_event');
  const id = identifier(transaction.transactionId), original = identifier(transaction.originalTransactionId);
  const from = integer(transaction.purchaseDate), until = integer(transaction.expiresDate);
  const signed = integer(transaction.signedDate), now = config.now?.() ?? Date.now();
  if (from > now + 300000 || signed > now + 300000 || until <= from || until - from > 400 * 86400000) throw new Error('invalid_apple_event');
  let renewalEnabled = null, version = signed;
  if (renewal) {
    if (renewal.environment !== config.environment || renewal.originalTransactionId !== original || renewal.productId !== config.productID ||
        (renewal.appAccountToken && renewal.appAccountToken.toLowerCase() !== transaction.appAccountToken.toLowerCase())) throw new Error('invalid_apple_event');
    if (![0, 1].includes(renewal.autoRenewStatus)) throw new Error('invalid_apple_event');
    renewalEnabled = renewal.autoRenewStatus === 1;
    version = Math.max(version, integer(renewal.signedDate));
  }
  if (notificationDate !== null) version = Math.max(version, integer(notificationDate));
  if (version > now + 300000) throw new Error('invalid_apple_event');
  const revoked = transaction.revocationDate == null ? null : integer(transaction.revocationDate);
  // A revoked subscription response must never be normalized as an active grant.
  if (status === 5 && revoked === null) throw new Error('invalid_apple_event');
  const event = {
    event_id: '', provider: 'apple', period_id: id, original_transaction_id: original,
    app_account_token: transaction.appAccountToken.toLowerCase(), product_id: config.productID,
    environment: config.environment, valid_from: Math.floor(from / 1000), valid_until: Math.floor(until / 1000),
    revoked_at: revoked === null ? null : Math.floor(revoked / 1000), event_version: version,
    renewal_enabled: renewalEnabled,
  };
  // Include normalized verified facts so same-millisecond renewal updates cannot
  // collide with a transaction delivery having unknown renewal status.
  event.event_id = eventID ?? 'transaction:' + createHash('sha256').update(JSON.stringify(event)).digest('hex');
  return event;
}

/** Dependencies must be an Apple SignedDataVerifier and AppStoreServerAPIClient. */
export function makeVerifier(verifier, appleAPI, config) {
  return async function verify(input) {
    if (!input || typeof input !== 'object' || Array.isArray(input)) throw new Error('invalid_request');
    if (input.kind === 'reconcile') {
      const original = identifier(input.original_transaction_id);
      const response = await appleAPI.getAllSubscriptionStatuses(original);
      if (response.environment !== config.environment || response.bundleId !== config.bundleID) throw new Error('invalid_apple_event');
      const entries = (response.data || []).flatMap(group => group.lastTransactions || []);
      if (entries.length > 32) throw new Error('invalid_apple_event');
      const events = [];
      for (const entry of entries) {
        const transaction = await verifier.verifyAndDecodeTransaction(entry.signedTransactionInfo);
        if (transaction.productId !== config.productID) continue;
        if (transaction.originalTransactionId !== original) continue;
        const renewal = entry.signedRenewalInfo ? await verifier.verifyAndDecodeRenewalInfo(entry.signedRenewalInfo) : null;
        events.push(normalizeTransaction(transaction, config, { renewal, status: entry.status }));
      }
      return { events };
    }
    if (!['transaction', 'notification'].includes(input.kind) || typeof input.signed_payload !== 'string' || input.signed_payload.length > 32768 || !input.signed_payload) throw new Error('invalid_request');
    if (input.kind === 'notification') {
      const notification = await verifier.verifyAndDecodeNotification(input.signed_payload);
      if (notification.notificationType === 'TEST') return { events: [] };
      // Refund reversals require explicit reconciliation/support policy; do not
      // resurrect terminal revoked periods from historical signed receipts.
      if (notification.notificationType === 'REFUND_REVERSED') return { events: [] };
      if (!notification.data?.signedTransactionInfo) return { events: [] };
      const transaction = await verifier.verifyAndDecodeTransaction(notification.data.signedTransactionInfo);
      const renewal = notification.data.signedRenewalInfo ? await verifier.verifyAndDecodeRenewalInfo(notification.data.signedRenewalInfo) : null;
      if (['REFUND', 'REVOKE'].includes(notification.notificationType) && transaction.revocationDate == null) throw new Error('invalid_apple_event');
      if (typeof notification.notificationUUID !== 'string' || !uuid.test(notification.notificationUUID)) throw new Error('invalid_apple_event');
      return { events: [normalizeTransaction(transaction, config, { renewal, notificationDate: notification.signedDate, eventID: 'notification:' + notification.notificationUUID })] };
    }
    // The submitted signature proves provenance, not the current refund state.
    const submitted = await verifier.verifyAndDecodeTransaction(input.signed_payload);
    normalizeTransaction(submitted, config);
    const current = await appleAPI.getTransactionInfo(identifier(submitted.transactionId));
    const transaction = await verifier.verifyAndDecodeTransaction(current.signedTransactionInfo);
    if (transaction.transactionId !== submitted.transactionId || transaction.originalTransactionId !== submitted.originalTransactionId || transaction.appAccountToken?.toLowerCase() !== submitted.appAccountToken?.toLowerCase()) throw new Error('invalid_apple_event');
    return { events: [normalizeTransaction(transaction, config)] };
  };
}
