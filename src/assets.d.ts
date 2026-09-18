// Files imported with { type: "file" }: the import is the path to read them from (inside a compiled binary, in /$bunfs).
declare module "*.ttf" {
  const path: string;
  export default path;
}
