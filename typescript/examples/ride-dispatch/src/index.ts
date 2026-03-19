import express, { type Request, type Response } from "express";

import { SpatiadClient } from "@spatiad/sdk";
import {
  spatiadWebhookJson,
  verifySpatiadWebhook
} from "@spatiad/express-plugin";

const client = new SpatiadClient("http://localhost:3000");

const run = async () => {
  const app = express();
  const webhookSecret = process.env.SPATIAD_WEBHOOK_SECRET ?? "dev-secret";

  app.post(
    "/webhooks/spatiad",
    spatiadWebhookJson(),
    verifySpatiadWebhook({ secret: webhookSecret }),
    (req: Request, res: Response) => {
      console.log("verified webhook payload", req.body);
      res.status(204).end();
    }
  );

  app.listen(4000, () => {
    console.log("example webhook server listening on :4000");
  });

  const response = await client.createOffer({
    jobId: "33333333-3333-3333-3333-333333333333",
    category: "tow_truck",
    pickup: { latitude: 38.433, longitude: 26.768 },
    dropoff: { latitude: 38.44, longitude: 26.78 },
    initialRadiusKm: 1,
    maxRadiusKm: 5,
    timeoutSeconds: 20
  });

  console.log("offer response", response);

  const events = await client.getJobEvents({
    jobId: "33333333-3333-3333-3333-333333333333",
    limit: 20,
    kinds: ["offer_created", "match_confirmed"]
  });

  console.log("filtered events", events);
};

run().catch((error) => {
  console.error("example failed", error);
  process.exitCode = 1;
});
