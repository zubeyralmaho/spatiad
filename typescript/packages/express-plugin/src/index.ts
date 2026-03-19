import type { RequestHandler } from "express";
import { SpatiadClient } from "@spatiad/sdk";

export type DispatchBridgeConfig = {
  spatiadBaseUrl: string;
};

export const attachSpatiadBridge = (config: DispatchBridgeConfig): RequestHandler => {
  const client = new SpatiadClient(config.spatiadBaseUrl);

  return (req, res, next) => {
    res.locals.spatiad = client;
    next();
  };
};
