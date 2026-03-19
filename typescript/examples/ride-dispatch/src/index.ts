import { SpatiadClient } from "@spatiad/sdk";

const client = new SpatiadClient("http://localhost:3000");

const run = async () => {
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
};

run().catch((error) => {
  console.error("example failed", error);
  process.exitCode = 1;
});
