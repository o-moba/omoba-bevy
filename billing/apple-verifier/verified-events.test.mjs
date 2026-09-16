import test from 'node:test';
import assert from 'node:assert/strict';
import { makeVerifier, normalizeTransaction } from './verified-events.mjs';
const config = { bundleID: 'test.omoba', productID: 'supporter.monthly', environment: 'Sandbox', now: () => 2000000 };
const tx = { bundleId: config.bundleID, environment: config.environment, productId: config.productID, type: 'Auto-Renewable Subscription', inAppOwnershipType: 'PURCHASED', appAccountToken: '01234567-1234-4234-8234-0123456789ab', transactionId: '123', originalTransactionId: '100', purchaseDate: 1000000, expiresDate: 4000000, signedDate: 2000000 };
test('strict product/account/environment/period policy applies after verification', () => {
  assert.equal(normalizeTransaction(tx, config).renewal_enabled, null);
  for (const [key, value] of [['productId','other'],['environment','Production'],['appAccountToken',''],['expiresDate',1],['signedDate',9999999999999],['inAppOwnershipType','FAMILY_SHARED']]) assert.throws(() => normalizeTransaction({ ...tx, [key]: value }, config));
});
test('renewal cancellation keeps paid expiry and is bound to the same subscription', () => {
  const renewal = { environment: 'Sandbox', originalTransactionId: '100', productId: config.productID, signedDate: 2000001, autoRenewStatus: 0 };
  const value = normalizeTransaction(tx, config, { renewal });
  assert.equal(value.renewal_enabled, false); assert.equal(value.valid_until, 4000); assert.equal(value.revoked_at, null);
  assert.throws(() => normalizeTransaction(tx, config, { renewal: { ...renewal, originalTransactionId: '101' } }));
});
test('receipt submission refreshes provider state and records refund before granting', async () => {
  const calls = [];
  const verifier = { verifyAndDecodeTransaction: async value => { calls.push(value); if(value === 'submitted') return tx; if(value === 'fresh') return {...tx,revocationDate:1500000}; throw new Error('invalid_signature'); } };
  const appleAPI = { getTransactionInfo: async id => { assert.equal(id,'123'); return {signedTransactionInfo:'fresh'}; } };
  const result = await makeVerifier(verifier,appleAPI,config)({kind:'transaction',signed_payload:'submitted'});
  assert.equal(result.events[0].revoked_at,1500); assert.deepEqual(calls,['submitted','fresh']);
  await assert.rejects(makeVerifier(verifier,appleAPI,config)({kind:'transaction',signed_payload:'forged'}));
});
test('verified notification requires separately verified nested transaction', async () => {
  let verified = false;
  const verifier = { verifyAndDecodeNotification: async () => ({notificationType:'REFUND',notificationUUID:'01234567-1234-4234-8234-0123456789ab',signedDate:2000001,data:{signedTransactionInfo:'nested'}}), verifyAndDecodeTransaction: async value => { assert.equal(value,'nested'); verified=true; return {...tx,revocationDate:1500000}; } };
  const result = await makeVerifier(verifier,{},config)({kind:'notification',signed_payload:'signed'});
  assert.equal(verified,true); assert.equal(result.events[0].revoked_at,1500);
});
test('reconciliation verifies latest provider periods and rejects a revoked response missing revocation', async () => {
  const verifier = { verifyAndDecodeTransaction: async () => tx };
  const appleAPI = { getAllSubscriptionStatuses: async () => ({bundleId:config.bundleID,environment:config.environment,data:[{lastTransactions:[{signedTransactionInfo:'signed',status:5}]}]}) };
  await assert.rejects(makeVerifier(verifier,appleAPI,config)({kind:'reconcile',original_transaction_id:'100'}));
});
