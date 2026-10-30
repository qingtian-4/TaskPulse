# License source material

`sources.json` records exact dependency versions, upstream revisions, original file URLs and SHA-256 digests. `upstream/` preserves the original license text or the explicitly identified license notice from those revisions. Notice extracts retain the hash of the full upstream source file as well as the extract hash.

`spdx/` contains unmodified canonical license terms from the recorded revision of the official SPDX license-list-data repository. These terms supplement an upstream declaration; they do not invent copyright attribution, replace an upstream notice or change a package's declared license expression. Cases that still need review remain visible in the generated inventory.

Native npm wrappers may share their parent's license files only when the package family, version and repository all match. System frameworks and native libraries embedded in third-party build tools are outside this inventory's independently enumerated scope.

The build generates `DEPENDENCIES.json`, `SBOM.cdx.json` and `THIRD_PARTY_NOTICES.txt` here for bundling. Those three generated reports are ignored by Git; the verified source material remains versioned so source archives can reproduce the notices. The application's own MIT license and original AgentTray attribution are included in the generated notices.
