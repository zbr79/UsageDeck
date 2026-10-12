import { getAccounts } from "@/lib/opencode";
import { requireLocalSecret } from "@/lib/secret";
import { NextRequest } from "next/server";

export const dynamic = "force-dynamic";

export async function GET(req: NextRequest) {
  const denied = await requireLocalSecret(req);
  if (denied) return denied;
  const email = req.nextUrl.searchParams.get("email") ?? "";
  const accounts = await getAccounts();
  const account = accounts.find((a) => a.email.toLowerCase() === email.toLowerCase());
  if (!account) {
    return Response.json({ error: "unknown account" }, { status: 404 });
  }
  return Response.json({ key: account.key });
}
