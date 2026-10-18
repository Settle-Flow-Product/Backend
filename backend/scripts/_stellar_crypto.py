"""
ed25519 and Stellar strkey, as a shared module.

Implemented inline because this machine has neither PyNaCl nor `cryptography`, and because the point
is to sign the way a WALLET would rather than to trust the same code the server verifies with. Two
implementations agreeing means something; one implementation agreeing with itself means nothing.

Extracted so `verify_wallet_auth.py` and `verify_api.py` cannot drift into signing differently.
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

import base64


def secret_to_seed(secret):
    """`S…` strkey to the 32-byte seed. The version byte leads, the checksum trails."""
    raw = base64.b32decode(secret + "=" * (-len(secret) % 8))
    return raw[1:33]


def sign_challenge(seed, transaction_xdr, network_passphrase):
    """
    Sign a SEP-10 challenge the way a wallet does, and return the signed envelope.

    The signature is over `sha256(network_id ‖ ENVELOPE_TYPE_TX ‖ tx_body)`, not over the envelope
    itself. Signing the wrong bytes produces a signature that is valid for something nobody asked
    about, which the server then rejects for reasons that look like a key problem.
    """
    envelope = base64.b64decode(transaction_xdr)
    assert envelope[-4:] == b"\x00\x00\x00\x00", "challenge arrived already signed"

    network_id = hashlib.sha256(network_passphrase.encode()).digest()
    tx_hash = hashlib.sha256(network_id + b"\x00\x00\x00\x02" + envelope[4:-4]).digest()

    pub = public_key(seed)
    sig = sign(seed, tx_hash)
    # A DecoratedSignature: the last four bytes of the public key as the hint, then the 64-byte
    # signature, both length-prefixed as XDR requires.
    signed = envelope[:-4] + b"\x00\x00\x00\x01" + pub[-4:] + b"\x00\x00\x00\x40" + sig
    return base64.b64encode(signed).decode()
