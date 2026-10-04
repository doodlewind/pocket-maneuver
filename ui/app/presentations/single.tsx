import { Text, View } from "@pocketjs/framework/components";
import { connectHost } from "../host.ts";

export default function SingleScreen() {
  const host = connectHost();
  return (
    <View class="w-full h-full items-center justify-center">
      <Text class="text-2xl font-bold text-white">{host.mode()}</Text>
    </View>
  );
}
