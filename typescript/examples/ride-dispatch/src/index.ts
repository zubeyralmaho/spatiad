import express, { type Request, type Response } from "express";
import { v4 as uuidv4 } from "uuid";

import { SpatiadClient } from "@spatiad/sdk";
import {
  spatiadWebhookJson,
  verifySpatiadWebhook
} from "@spatiad/express-plugin";

const client = new SpatiadClient("http://localhost:3000");

// Simulate multiple drivers with location updates
interface Driver {
  id: string;
  latitude: number;
  longitude: number;
  category: string;
}

const drivers: Driver[] = [
  { id: uuidv4(), latitude: 38.433, longitude: 26.768, category: "tow_truck" },
  { id: uuidv4(), latitude: 38.435, longitude: 26.769, category: "tow_truck" },
  { id: uuidv4(), latitude: 38.431, longitude: 26.767, category: "tow_truck" },
  { id: uuidv4(), latitude: 38.436, longitude: 26.770, category: "tow_truck" },
];

async function registerDrivers() {
  console.log("\n📍 Registering drivers...");
  for (const driver of drivers) {
    await client.upsertDriver({
      driverId: driver.id,
      category: driver.category,
      status: "Available",
      position: { latitude: driver.latitude, longitude: driver.longitude },
    });
    console.log(`  ✓ Driver ${driver.id.slice(0, 8)} at (${driver.latitude}, ${driver.longitude})`);
  }
}

async function demonstrateOfferCreation() {
  console.log("\n🚗 Creating offer for job...");
  
  const jobId = uuidv4();
  const response = await client.createOffer({
    jobId,
    category: "tow_truck",
    pickup: { latitude: 38.433, longitude: 26.768 },
    dropoff: { latitude: 38.44, longitude: 26.78 },
    initialRadiusKm: 0.5,
    maxRadiusKm: 5,
    timeoutSeconds: 30,
    retry: {
      maxAttempts: 3,
      backoffMs: 100
    }
  });

  console.log(`  ✓ Offer created: ${response.offer_id}`);
  return jobId;
}

async function demonstrateRadiusExpansion() {
  console.log("\n📡 Testing radius expansion...");
  
  // Create a driver far away
  const farDriver = {
    id: uuidv4(),
    latitude: 38.45,
    longitude: 26.80,
    category: "tow_truck"
  };
  
  await client.upsertDriver({
    driverId: farDriver.id,
    category: farDriver.category,
    status: "Available",
    position: { latitude: farDriver.latitude, longitude: farDriver.longitude },
  });
  console.log(`  ✓ Registered far driver at (${farDriver.latitude}, ${farDriver.longitude})`);

  const jobId = uuidv4();
  const response = await client.createOffer({
    jobId,
    category: "tow_truck",
    pickup: { latitude: 38.433, longitude: 26.768 },
    dropoff: { latitude: 38.44, longitude: 26.78 },
    initialRadiusKm: 0.1,  // Very small initial radius
    maxRadiusKm: 5,        // Will expand to find the far driver
    timeoutSeconds: 30,
    retry: {
      maxAttempts: 3,
      backoffMs: 100
    }
  });

  console.log(`  ✓ Offer created via radius expansion: ${response.offer_id}`);
  return jobId;
}

async function demonstrateJobStatus(jobId: string) {
  console.log("\n📊 Checking job status...");
  
  const status = await client.getJobStatus({
    jobId,
  });

  console.log(`  ✓ Job ${jobId.slice(0, 8)}`);
  console.log(`    State: ${status.state}`);
  console.log(`    Matched Driver: ${status.matched_driver_id || "none"}`);
  console.log(`    Matched Offer: ${status.matched_offer_id || "none"}`);
}

async function demonstrateJobEvents(jobId: string) {
  console.log("\n📋 Fetching job events...");
  
  const events = await client.getJobEvents({
    jobId,
    limit: 50,
    kinds: ["job_registered", "offer_created", "match_confirmed"]
  });

  if (events.events.length === 0) {
    console.log(`  ℹ️  No events found`);
    return;
  }

  console.log(`  ✓ Found ${events.events.length} event(s):`);
  for (const event of events.events) {
    console.log(`    - ${event.kind} at ${event.at}`);
  }
}

async function demonstrateInputValidation() {
  console.log("\n⚠️  Testing input validation...");
  
  const testCases = [
    {
      name: "Invalid category (too long)",
      config: {
        jobId: uuidv4(),
        category: "a".repeat(51),  // > 50 chars
        pickup: { latitude: 38.433, longitude: 26.768 },
        initialRadiusKm: 1,
        maxRadiusKm: 5,
        timeoutSeconds: 20,
      }
    },
    {
      name: "Invalid coordinates (out of range)",
      config: {
        jobId: uuidv4(),
        category: "tow_truck",
        pickup: { latitude: 91, longitude: 26.768 },  // > 90
        initialRadiusKm: 1,
        maxRadiusKm: 5,
        timeoutSeconds: 20,
      }
    },
    {
      name: "Invalid radius (initial > max)",
      config: {
        jobId: uuidv4(),
        category: "tow_truck",
        pickup: { latitude: 38.433, longitude: 26.768 },
        initialRadiusKm: 10,
        maxRadiusKm: 5,  // < initial
        timeoutSeconds: 20,
      }
    },
    {
      name: "Invalid timeout (0 seconds)",
      config: {
        jobId: uuidv4(),
        category: "tow_truck",
        pickup: { latitude: 38.433, longitude: 26.768 },
        initialRadiusKm: 1,
        maxRadiusKm: 5,
        timeoutSeconds: 0,  // Invalid
      }
    },
  ];

  for (const testCase of testCases) {
    try {
      // @ts-ignore - intentionally passing invalid config
      await client.createOffer(testCase.config);
      console.log(`  ✗ ${testCase.name}: Should have failed but didn't`);
    } catch (error) {
      const err = error as any;
      console.log(`  ✓ ${testCase.name}: Rejected`);
      if (err.message) {
        console.log(`    Error: ${err.message.slice(0, 60)}...`);
      }
    }
  }
}

async function demonstrateOffersAndCancellation() {
  console.log("\n❌ Testing offer/job operations...");

  const jobId = uuidv4();
  
  // Create offer
  const offerResponse = await client.createOffer({
    jobId,
    category: "tow_truck",
    pickup: { latitude: 38.433, longitude: 26.768 },
    dropoff: { latitude: 38.44, longitude: 26.78 },
    initialRadiusKm: 1,
    maxRadiusKm: 5,
    timeoutSeconds: 30,
  });

  console.log(`  ✓ Offer created: ${offerResponse.offer_id?.slice(0, 8)}`);

  // Check job status is pending/searching
  const statusBefore = await client.getJobStatus({ jobId });
  console.log(`  ✓ Job state before: ${statusBefore.state}`);

  // Try to cancel offer
  if (offerResponse.offer_id && offerResponse.offer_id !== "00000000-0000-0000-0000-000000000000") {
    await client.cancelOffer({ offerId: offerResponse.offer_id });
    console.log(`  ✓ Offer cancelled`);
  }

  // Check status after
  const statusAfter = await client.getJobStatus({ jobId });
  console.log(`  ✓ Job state after: ${statusAfter.state}`);
}

const run = async () => {
  const app = express();
  const webhookSecret = process.env.SPATIAD_WEBHOOK_SECRET ?? "dev-secret";

  // Webhook receiver
  app.post(
    "/webhooks/spatiad",
    spatiadWebhookJson(),
    verifySpatiadWebhook({ secret: webhookSecret }),
    (req: Request, res: Response) => {
      console.log("\n✅ Webhook received:", {
        event: (req.body as any).event,
        jobId: (req.body as any).job_id?.slice(0, 8),
        driverId: (req.body as any).driver_id?.slice(0, 8),
      });
      res.status(204).end();
    }
  );

  app.listen(4000, () => {
    console.log("🎯 Webhook receiver listening on :4000");
  });

  try {
    console.log("═══════════════════════════════════════");
    console.log("  Spatiad Demo - Multi-Driver Dispatch");
    console.log("═══════════════════════════════════════");

    // Run all demonstrations
    await registerDrivers();
    
    const jobId = await demonstrateOfferCreation();
    await demonstrateJobStatus(jobId);
    await demonstrateJobEvents(jobId);

    await demonstrateRadiusExpansion();
    
    await demonstrateInputValidation();
    
    await demonstrateOffersAndCancellation();

    console.log("\n═══════════════════════════════════════");
    console.log("  Demo Completed Successfully ✓");
    console.log("═══════════════════════════════════════\n");

  } catch (error) {
    console.error("❌ Demo failed:", error);
  } finally {
    process.exit(0);
  }
};

run().catch((error) => {
  console.error("❌ Example failed", error);
  process.exitCode = 1;
});
