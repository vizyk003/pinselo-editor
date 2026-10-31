`sRGB-v4.icc`: the compact sRGB v4 profile from https://github.com/saucecontrol/Compact-ICC-Profiles (CC0 1.0, see
`LICENSE-CC0`). It is the RGB working/output profile of the bundled registry (`packages/schema/src/icc-profiles.json`,
id `srgb`). The CMYK print profiles are not committed: `pnpm icc:sync` downloads them from eci.org into `fixtures/icc/`.
