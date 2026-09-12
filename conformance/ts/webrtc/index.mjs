// H6 close-candidate remainder: SIGNAL + DataChannel carrying the shared
// RELAY 0.2 peer protocol (reconnect/resume + admission + HELLO.datastore
// in AuthTranscript). Not a crate. Not H6 closed. Not M4a complete.

export { CHANNEL_LABEL, ChannelTransport, FakeDataChannel, FakeRTCPeerConnection, connectFakeRtc, pairDataChannels } from './channel.mjs'
export { ERR_TARGET_NOT_CONNECTED, SignalRelay, encodeError, encodeSignal, signalingBytes, signalingObject } from './signal.mjs'
export {
  AUTH_WRONG_DATASTORE,
  ERR_AUTH_FAILED,
  ERR_AUTH_WRONG_DATASTORE,
  ERR_VERSION_MISMATCH,
  admitDatastore,
  connectDirect,
  runNegotiated,
  serveDirect,
  signAuthV1NonceOnly,
} from './peer.mjs'
export { negotiateViaSignal } from './negotiate.mjs'
