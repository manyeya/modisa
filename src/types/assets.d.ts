declare module "*.toml" {
  const value: any;
  export default value;
}
declare module "*.dylib" {
  const path: string;
  export default path;
}
declare module "*.so" {
  const path: string;
  export default path;
}
declare module "*.md" {
  const text: string;
  export default text;
}
