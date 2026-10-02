import { renamed, usedA, type Shape } from "@fixture/lib";
import * as format from "@/app/format";
import { api } from "@/convex/_generated/api";

export const metadata = { title: "Dashboard" };

export default function Page(props: { shape: Shape }) {
  const send = api.messages.send;
  return `${usedA()} ${renamed()} ${format.money(1)} ${props.shape.id} ${String(send)}`;
}
