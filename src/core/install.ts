// How this shepherd got here, judged by where its binary lives. A package manager owns the binary it
// installed, so updating or removing that binary is the manager's job: shepherd only says which command.
export type Install = { by: "source" | "script" | "homebrew" | "mise" | "system"; manager?: string; upgrade?: string; remove?: string };

export function installedBy(exe: string, fromSource: boolean): Install {
  if (fromSource) return { by: "source" };
  if (/\/(Cellar|homebrew|linuxbrew)\//.test(exe)) return { by: "homebrew", manager: "Homebrew", upgrade: "brew upgrade shepherd", remove: "brew uninstall shepherd" };
  if (exe.includes("/mise/installs/")) return { by: "mise", manager: "mise", upgrade: "mise upgrade github:manyeya/shepherd", remove: "mise uninstall github:manyeya/shepherd" };
  // .deb and .rpm packages put it in /usr/bin
  if (/^\/usr\/s?bin\//.test(exe)) return { by: "system", manager: "your system's package manager", upgrade: "sudo apt upgrade shepherd (or dnf upgrade shepherd)", remove: "sudo apt remove shepherd (or dnf remove shepherd)" };
  return { by: "script" }; // install.sh, or a binary someone put on their PATH by hand
}
