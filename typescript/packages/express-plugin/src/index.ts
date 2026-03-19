import crypto from "node:crypto";

import express, { type Request, type RequestHandler } from "express";
import { SpatiadClient } from "@spatiad/sdk";

export type DispatchBridgeConfig = {
  spatiadBaseUrl: string;
};

export type WebhookVerificationConfig = {
  secret: string;
  maxSkewSeconds?: number;
  nonceTtlSeconds?: number;
  now?: () => number;
};

type RequestWithRawBody = Request & { rawBody?: string };

const usedNonceAt = new Map<string, number>();

export const spatiadWebhookJson = (): RequestHandler => {
  return express.json({
    verify: (req, _res, buf) => {
      (req as RequestWithRawBody).rawBody = buf.toString("utf8");
    }
  });
};

export const verifySpatiadWebhook = (config: WebhookVerificationConfig): RequestHandler => {
  const maxSkewSeconds = config.maxSkewSeconds ?? 300;
  const nonceTtlSeconds = config.nonceTtlSeconds ?? 600;
  const now = config.now ?? (() => Math.floor(Date.now() / 1000));

  return (req, res, next) => {
    const timestampHeader = req.header("x-spatiad-timestamp");
    const nonce = req.header("x-spatiad-nonce");
    const signature = req.header("x-spatiad-signature");

    if (!timestampHeader || !nonce || !signature) {
      res.status(401).json({ error: "missing webhook auth headers" });
      return;
    }

    const timestamp = Number.parseInt(timestampHeader, 10);
    if (!Number.isFinite(timestamp)) {
      res.status(401).json({ error: "invalid timestamp header" });
      return;
    }

    const nowTs = now();
    if (Math.abs(nowTs - timestamp) > maxSkewSeconds) {
      res.status(401).json({ error: "timestamp skew exceeded" });
      return;
    }

    cleanupExpiredNonces(nowTs, nonceTtlSeconds);
    if (usedNonceAt.has(nonce)) {
      res.status(401).json({ error: "replayed nonce" });
      return;
    }

    const rawBody = (req as RequestWithRawBody).rawBody ?? JSON.stringify(req.body ?? {});
    const payloadToSign = `${timestampHeader}.${nonce}.${rawBody}`;

    const expected = crypto
      .createHmac("sha256", config.secret)
      .update(payloadToSign)
      .digest("hex");

    const provided = signature.trim().toLowerCase();
    if (!timingSafeEqualHex(expected, provided)) {
      res.status(401).json({ error: "invalid webhook signature" });
      return;
    }

    usedNonceAt.set(nonce, nowTs);
    next();
  };
};

function cleanupExpiredNonces(nowTs: number, nonceTtlSeconds: number): void {
  for (const [nonce, seenAt] of usedNonceAt.entries()) {
    if (nowTs - seenAt > nonceTtlSeconds) {
      usedNonceAt.delete(nonce);
    }
  }
}

function timingSafeEqualHex(a: string, b: string): boolean {
  if (a.length !== b.length) {
    return false;
  }

  const aBuf = Buffer.from(a, "utf8");
  const bBuf = Buffer.from(b, "utf8");
  return crypto.timingSafeEqual(aBuf, bBuf);
}

export const attachSpatiadBridge = (config: DispatchBridgeConfig): RequestHandler => {
  const client = new SpatiadClient(config.spatiadBaseUrl);

  return (req, res, next) => {
    res.locals.spatiad = client;
    next();
  };
};
