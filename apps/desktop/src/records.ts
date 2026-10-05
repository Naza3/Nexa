/** Model IDs such as constructor are valid. Never read inherited object members. */
export function ownRecord<T>(record: Record<string, T>, key: string): T | undefined {
  return Object.hasOwn(record, key) ? record[key] : undefined;
}
