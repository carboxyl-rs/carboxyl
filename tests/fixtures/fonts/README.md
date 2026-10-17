These Latin subsets of Noto Sans and Noto Sans Mono are bundled so the rendering
fixture works on headless hosts without system fonts. Both are licensed under
the SIL Open Font License; their license files are included here.

`fonts.conf` registers the TrueType faces in `system/` with Fontconfig, including
generic serif, sans-serif and monospace aliases. CI sets `FONTCONFIG_FILE` to this
file and checks both font matches before starting Servo. Web font files alone
are insufficient on a headless host with no system font families.

Sources (Google Fonts):

- Noto Sans: https://fonts.gstatic.com/s/notosans/v42/o-0bIpQlx3QUlC5A4PNB6Ryti20_6n1iPHjc5a7duw.woff2
- Noto Sans Mono: https://fonts.gstatic.com/s/notosansmono/v37/BngrUXNETWXI6LwhGYvaxZikqZqK6fBq6kPvUce2oAZcdthSBUsYck4-_FNJ093dVQ.woff2
- Licenses: https://github.com/google/fonts/tree/main/ofl/notosans and https://github.com/google/fonts/tree/main/ofl/notosansmono
- Noto Sans TrueType: https://fonts.gstatic.com/s/notosans/v42/o-0mIpQlx3QUlC5A4PNB6Ryti20_6n1iPHjcz6L1SoM-jCpoiyD9A99d.ttf
- Noto Sans Mono TrueType: https://fonts.gstatic.com/s/notosansmono/v37/BngrUXNETWXI6LwhGYvaxZikqZqK6fBq6kPvUce2oAZcdthSBUsYck4-_FNJ49o.ttf
