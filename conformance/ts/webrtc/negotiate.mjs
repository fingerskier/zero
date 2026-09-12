// SIGNAL-facilitated fake RTC: initiator offer DataChannel, answerer
// receives ondatachannel. Payloads are opaque JSON bytes.
// RTC offerer/answerer is not the DataChannel handshake role —
// that is `isHandshakeServer` (PeerId order) after the channel is up.

import { decodeEnvelope } from '../models/relay.mjs'
import {
  FakeRTCPeerConnection,
  connectFakeRtc,
} from './channel.mjs'
import { channelBindingFor } from './binding.mjs'
import { encodeSignal, signalingBytes, signalingObject } from './signal.mjs'

export async function negotiateViaSignal(relay, initiatorId, answererId) {
  const offerSess = relay.connect(initiatorId)
  const answerSess = relay.connect(answererId)
  const offerer = new FakeRTCPeerConnection()
  const answerer = new FakeRTCPeerConnection()
  const channel = offerer.createDataChannel('zerodb-relay', { ordered: true })

  const iceOut = []
  offerer.onicecandidate = (ev) => {
    if (!ev.candidate) return
    iceOut.push({ from: initiatorId, to: answererId, candidate: ev.candidate })
  }
  answerer.onicecandidate = (ev) => {
    if (!ev.candidate) return
    iceOut.push({ from: answererId, to: initiatorId, candidate: ev.candidate })
  }

  const offer = await offerer.createOffer()
  await offerer.setLocalDescription(offer)
  const offerErr = relay.handle(
    initiatorId,
    encodeSignal(10, { target: answererId, payload: signalingBytes({ kind: 'offer', sdp: offer }) }),
  )
  if (offerErr) return { error: decodeEnvelope(offerErr).payload, offerer, answerer, channel }

  const offerFrame = decodeEnvelope(await answerSess.recv())
  const offerObj = signalingObject(offerFrame.payload.payload)
  await answerer.setRemoteDescription(offerObj.sdp)
  const answer = await answerer.createAnswer()
  await answerer.setLocalDescription(answer)
  const answerErr = relay.handle(
    answererId,
    encodeSignal(11, { target: initiatorId, payload: signalingBytes({ kind: 'answer', sdp: answer }) }),
  )
  if (answerErr) return { error: decodeEnvelope(answerErr).payload, offerer, answerer, channel }

  const answerFrame = decodeEnvelope(await offerSess.recv())
  const answerObj = signalingObject(answerFrame.payload.payload)
  await offerer.setRemoteDescription(answerObj.sdp)

  await new Promise((resolve) => setTimeout(resolve, 0))
  for (const ice of iceOut) {
    const err = relay.handle(
      ice.from,
      encodeSignal(12, {
        target: ice.to,
        payload: signalingBytes({ kind: 'ice', candidate: ice.candidate }),
      }),
    )
    if (err) return { error: decodeEnvelope(err).payload, offerer, answerer, channel }
    const dest = ice.to === initiatorId ? offerSess : answerSess
    const iceFrame = decodeEnvelope(await dest.recv())
    const iceObj = signalingObject(iceFrame.payload.payload)
    const pc = ice.to === initiatorId ? offerer : answerer
    await pc.addIceCandidate(iceObj.candidate)
  }

  const pair = connectFakeRtc(offerer, answerer)
  // Each side derives HELLO.channel_binding from its own local + remote
  // SDP fingerprints. Honest signaling ⇒ equal; a bridging MITM ⇒ not.
  return {
    offerer,
    answerer,
    initiatorChannel: pair.initiator,
    answererChannel: pair.answerer,
    initiatorBinding: channelBindingFor(offerer),
    answererBinding: channelBindingFor(answerer),
    /** `opts.pc` for runNegotiated, by peer id (hex). */
    pcFor: (peerId) => (String(peerId).toLowerCase() === String(initiatorId).toLowerCase() ? offerer : answerer),
    offerSession: offerSess,
    answerSession: answerSess,
  }
}
