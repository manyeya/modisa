// How this modisa got here, judged by where its binary lives. A package manager owns the binary it
// installed, so updating or removing that binary is the manager's job: modisa only says which command.
export type Install = { by: "source" | "script" | "homebrew" | "mise" | "system"; manager?: string; upgrade?: string; remove?: string };

export function installedBy(exe: string, fromSource: boolean): Install {
  if (fromSource) return { by: "source" };
  if (/\/(Cellar|homebrew|linuxbrew)\//.test(exe)) return { by: "homebrew", manager: "Homebrew", upgrade: "brew upgrade modisa", remove: "brew uninstall modisa" };
  if (exe.includes("/mise/installs/")) return { by: "mise", manager: "mise", upgrade: "mise upgrade github:manyeya/modisa", remove: "mise uninstall github:manyeya/modisa" };
  // .deb and .rpm packages put it in /usr/bin
  if (/^\/usr\/s?bin\//.test(exe)) return { by: "system", manager: "your system's package manager", upgrade: "sudo apt upgrade modisa (or dnf upgrade modisa)", remove: "sudo apt remove modisa (or dnf remove modisa)" };
  return { by: "script" }; // install.sh, or a binary someone put on their PATH by hand
}
