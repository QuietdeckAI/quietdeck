// The deck (webauthn-rs) sends options with base64url strings where the browser wants ArrayBuffers, and wants the
// credential back the same way. These convert both directions.

function fromB64url(s: string): ArrayBuffer {
  const b64 = s.replace(/-/g, "+").replace(/_/g, "/").padEnd(Math.ceil(s.length / 4) * 4, "=");
  const bin = atob(b64);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out.buffer;
}

function toB64url(buf: ArrayBuffer | null): string | null {
  if (!buf) return null;
  const bytes = new Uint8Array(buf);
  let bin = "";
  for (const b of bytes) bin += String.fromCharCode(b);
  return btoa(bin).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

type Descriptor = { id: string; type: string; transports?: string[] };
type CreationOptions = {
  publicKey: Omit<PublicKeyCredentialCreationOptions, "challenge" | "user" | "excludeCredentials"> & {
    challenge: string;
    user: { id: string; name: string; displayName: string };
    excludeCredentials?: Descriptor[];
  };
};
type RequestOptions = {
  publicKey: Omit<PublicKeyCredentialRequestOptions, "challenge" | "allowCredentials"> & {
    challenge: string;
    allowCredentials?: Descriptor[];
  };
};

const descriptor = (d: Descriptor): PublicKeyCredentialDescriptor => ({
  ...d,
  id: fromB64url(d.id),
  type: "public-key",
  transports: d.transports as AuthenticatorTransport[] | undefined,
});

export function passkeysSupported(): boolean {
  return typeof window.PublicKeyCredential === "function" && !!navigator.credentials;
}

export async function createPasskey(options: unknown): Promise<unknown> {
  const { publicKey } = options as CreationOptions;
  const cred = (await navigator.credentials.create({
    publicKey: {
      ...publicKey,
      challenge: fromB64url(publicKey.challenge),
      user: { ...publicKey.user, id: fromB64url(publicKey.user.id) },
      excludeCredentials: publicKey.excludeCredentials?.map(descriptor),
    },
  })) as PublicKeyCredential | null;
  if (!cred) throw new Error("No passkey was created.");
  const response = cred.response as AuthenticatorAttestationResponse;
  return {
    id: cred.id,
    rawId: toB64url(cred.rawId),
    type: cred.type,
    response: {
      attestationObject: toB64url(response.attestationObject),
      clientDataJSON: toB64url(response.clientDataJSON),
      transports: response.getTransports?.() ?? [],
    },
    extensions: cred.getClientExtensionResults(),
  };
}

export async function getPasskey(options: unknown): Promise<unknown> {
  const { publicKey } = options as RequestOptions;
  const cred = (await navigator.credentials.get({
    publicKey: {
      ...publicKey,
      challenge: fromB64url(publicKey.challenge),
      allowCredentials: publicKey.allowCredentials?.map(descriptor),
    },
  })) as PublicKeyCredential | null;
  if (!cred) throw new Error("No passkey was chosen.");
  const response = cred.response as AuthenticatorAssertionResponse;
  return {
    id: cred.id,
    rawId: toB64url(cred.rawId),
    type: cred.type,
    response: {
      authenticatorData: toB64url(response.authenticatorData),
      clientDataJSON: toB64url(response.clientDataJSON),
      signature: toB64url(response.signature),
      userHandle: toB64url(response.userHandle),
    },
    extensions: cred.getClientExtensionResults(),
  };
}

/** A name for this device's passkey, from the browser's own description of itself. */
export function guessDeviceName(): string {
  const ua = navigator.userAgent;
  const os = /iPhone/.test(ua)
    ? "iPhone"
    : /iPad/.test(ua)
      ? "iPad"
      : /Android/.test(ua)
        ? "Android"
        : /Mac OS X/.test(ua)
          ? "Mac"
          : /Windows/.test(ua)
            ? "Windows"
            : /Linux/.test(ua)
              ? "Linux"
              : "This device";
  const browser = /Edg\//.test(ua)
    ? "Edge"
    : /Firefox\//.test(ua)
      ? "Firefox"
      : /Chrome\//.test(ua)
        ? "Chrome"
        : /Safari\//.test(ua)
          ? "Safari"
          : "";
  return browser ? `${os} · ${browser}` : os;
}
