"""
Prove SEP-10 sign-in end to end against the running backend.

Wallet auth failed twice in this project — once because a refusal path logged nothing, once because
`signMessage` is not interoperable. This exercises the shipped path over real HTTP with a real
ed25519 signature, so a regression in the challenge, the hash, or the verifier is caught here rather
than by somebody clicking a button.

ed25519 is implemented inline: this machine has neither PyNaCl nor `cryptography`, and the point is
to sign the way a wallet would, not to trust the same code the server verifies with.
"""
import hashlib, json, os, struct, urllib.request

# --- ed25519 (RFC 8032), reference arithmetic ---------------------------------------------------
q = 2**255 - 19
L = 2**252 + 27742317777372353535851937790883648493
d = -121665 * pow(121666, q - 2, q) % q
I = pow(2, (q - 1) // 4, q)

def inv(x): return pow(x, q - 2, q)

def edwards_add(P, Q):
    x1, y1, z1, t1 = P
    x2, y2, z2, t2 = Q
    a = (y1 - x1) * (y2 - x2) % q
    b = (y1 + x1) * (y2 + x2) % q
    c = t1 * 2 * d * t2 % q
    dd = z1 * 2 * z2 % q
    e, f, g, h = b - a, dd - c, dd + c, b + a
    return (e * f % q, g * h % q, f * g % q, e * h % q)

def edwards_double(P):
    return edwards_add(P, P)

def scalarmult(P, e):
    Q = (0, 1, 1, 0)
    while e > 0:
        if e & 1: Q = edwards_add(Q, P)
        P = edwards_double(P)
        e >>= 1
    return Q

def xrecover(y):
    xx = (y * y - 1) * inv(d * y * y + 1)
    x = pow(xx, (q + 3) // 8, q)
    if (x * x - xx) % q != 0: x = x * I % q
    if x % 2 != 0: x = q - x
    return x

By = 4 * inv(5) % q
Bx = xrecover(By)
B = (Bx % q, By % q, 1, Bx * By % q)

def encode_point(P):
    x, y, z, _ = P
    zi = inv(z)
    x, y = x * zi % q, y * zi % q
    return int.to_bytes(y | ((x & 1) << 255), 32, 'little')

def secret_expand(sk):
    h = hashlib.sha512(sk).digest()
    a = int.from_bytes(h[:32], 'little')
    a &= (1 << 254) - 8
    a |= 1 << 254
    return a, h[32:]

def public_key(sk):
    a, _ = secret_expand(sk)
    return encode_point(scalarmult(B, a))

def sign(sk, msg):
    a, prefix = secret_expand(sk)
    A = encode_point(scalarmult(B, a))
    r = int.from_bytes(hashlib.sha512(prefix + msg).digest(), 'little') % L
    R = encode_point(scalarmult(B, r))
    k = int.from_bytes(hashlib.sha512(R + A + msg).digest(), 'little') % L
    return R + int.to_bytes((r + k * a) % L, 32, 'little')

# --- strkey -------------------------------------------------------------------------------------
def crc16(data):
    crc = 0
    for b in data:
        crc ^= b << 8
        for _ in range(8):
            crc = ((crc << 1) ^ 0x1021) & 0xFFFF if crc & 0x8000 else (crc << 1) & 0xFFFF
    return crc

def strkey(version_byte, payload):
    import base64
    body = bytes([version_byte]) + payload
    return base64.b32encode(body + struct.pack('<H', crc16(body))).decode().rstrip('=')

# --- the exercise -------------------------------------------------------------------------------
import base64
BASE = os.environ.get('BASE', 'http://localhost:8188')
seed = hashlib.sha256(b'settleflow sep-10 end to end').digest()
pub = public_key(seed)
address = strkey(6 << 3, pub)          # 'G'
print(f'  wallet            : {address}')

def post(path, body):
    req = urllib.request.Request(BASE + path, data=json.dumps(body).encode(),
                                 headers={'content-type': 'application/json'})
    try:
        with urllib.request.urlopen(req, timeout=30) as r:
            return r.status, json.load(r)
    except urllib.error.HTTPError as e:
        return e.code, json.load(e)

status, ch = post('/auth/wallet/challenge', {'address': address})
assert status == 200, (status, ch)
print(f'  challenge         : nonce {ch["nonce"][:16]}… domain {ch["domain"]}')

envelope = base64.b64decode(ch['transaction_xdr'])
assert envelope[:4] == b'\x00\x00\x00\x02', 'not an ENVELOPE_TYPE_TX envelope'
assert envelope[-4:] == b'\x00\x00\x00\x00', 'challenge arrived already signed'
tx_body = envelope[4:-4]

network_id = hashlib.sha256(ch['network_passphrase'].encode()).digest()
payload = network_id + b'\x00\x00\x00\x02' + tx_body
tx_hash = hashlib.sha256(payload).digest()

sig = sign(seed, tx_hash)
signed = envelope[:-4] + b'\x00\x00\x00\x01' + pub[-4:] + b'\x00\x00\x00\x40' + sig
signed_xdr = base64.b64encode(signed).decode()

status, body = post('/auth/wallet/verify', {'nonce': ch['nonce'], 'signed_xdr': signed_xdr})
print(f'  verify            : {status} {"token issued" if status == 200 and body.get("token") else body}')
ok = status == 200 and bool(body.get('token'))

# The same challenge must not verify twice: the nonce is single-use.
status2, body2 = post('/auth/wallet/verify', {'nonce': ch['nonce'], 'signed_xdr': signed_xdr})
replay_refused = status2 != 200
print(f'  replayed nonce    : {status2} {"refused" if replay_refused else "ACCEPTED — REPLAYABLE"}')

# A signature from a different key over the same challenge must not verify.
status3, ch3 = post('/auth/wallet/challenge', {'address': address})
env3 = base64.b64decode(ch3['transaction_xdr'])
other = hashlib.sha256(b'somebody else entirely').digest()
h3 = hashlib.sha256(network_id + b'\x00\x00\x00\x02' + env3[4:-4]).digest()
forged = env3[:-4] + b'\x00\x00\x00\x01' + public_key(other)[-4:] + b'\x00\x00\x00\x40' + sign(other, h3)
status4, _ = post('/auth/wallet/verify',
                  {'nonce': ch3['nonce'], 'signed_xdr': base64.b64encode(forged).decode()})
wrong_key_refused = status4 != 200
print(f'  wrong key         : {status4} {"refused" if wrong_key_refused else "ACCEPTED — FORGEABLE"}')

raise SystemExit(0 if (ok and replay_refused and wrong_key_refused) else 1)
