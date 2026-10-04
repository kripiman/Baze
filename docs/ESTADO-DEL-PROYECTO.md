<!--
SPDX-FileCopyrightText: 2026 Gabriel Piñones
SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Estado del proyecto Baze — traspaso

Documento de traspaso: **qué se hizo, qué falta y cómo retomar** el trabajo sin depender de la conversación que lo produjo. Se actualiza al cerrar cada etapa (ver [Registro de avance](#6-registro-de-avance-de-la-fase-actual)). La fuente canónica de reglas del repo sigue siendo [AGENTS.md](../AGENTS.md).

- **Rama de trabajo**: `FirstVersion` (no existe ningún Pull Request; no se abre ninguno sin petición expresa).
- **Origen**: auditoría de seguridad y funcionalidad (19 hallazgos: SEC-01…12, FUN-01…07) y plan de remediación por rondas.
- **Última actualización**: ver la fecha del último commit que toca este archivo (`git log -1 -- docs/ESTADO-DEL-PROYECTO.md`).

---

## 1. Resumen

1. La **Ronda 1** (seguridad, persistencia real en PostGIS, infraestructura endurecida, CI verde, Android compilando, documentación) y la fase **0 + A + B** (ruteo y búsqueda reales, pipeline de datos y motores) están **implementadas, empujadas y verificadas en el CI real**.
2. El backend persiste cuentas, reportes y votos en PostgreSQL/PostGIS (migraciones, rol sin privilegios, purga, topes anti-abuso) y **rutea de verdad**: pide la ruta a Valhalla, comprueba los cierres confirmados en PostGIS y repite la petición con `exclude_polygons` hasta que la ruta no cruce ninguno. Es **fail-closed**: nunca entrega una ruta que cruce un cierre confirmado (`404` si no hay alternativa, `503` si hay demasiados cierres).
3. Ruteo y búsqueda están **apagados por defecto** (`ENGINES_ENABLED=false` ⇒ `501`): un despliegue debe construir antes los datos de su región y levantar los motores (`docs/operations.md`, sección 5.1).
4. El pipeline de datos (`data/scripts/`) descarga y verifica el extracto, construye las teselas, un grafo de Valhalla **con elevación obligatoria** y el índice de Photon, y publica el mapa con un manifiesto (tamaño y sha256). `data-smoke.yml` lo ejecuta con motores reales sobre Mónaco, cada semana y en cada cambio.
5. La app Android **compila y pasa lint, pruebas y R8**, pero sigue siendo un andamiaje: no es funcional todavía (Etapa C, **no autorizada**).
6. Falta, además de la app, lo que solo puede decidir el mantenedor (sección 4.4): región y fuente de Photon para el despliegue, CLA, buzón de seguridad, dominio.

---

## 2. Reglas de entrega (decisiones del mantenedor)

| Tema | Regla |
|---|---|
| Ramas | Todo en `FirstVersion`, empujado a esa rama. Sin otras ramas ni PRs salvo petición. **Nunca `force push`**. No tocar la configuración git del repo. |
| Commits | Grupos de commits convencionales (`feat(...)`, `fix(...)`, `test(...)`, `docs(...)`, `ci(...)`…), uno por unidad lógica. Mensajes en inglés. |
| Identidad | Autor y committer `gabriel <131714613+gabrielpinones@users.noreply.github.com>` y trailer `Co-authored-by: kripiman <119073556+kripiman@users.noreply.github.com>`. **Sin atribución a Claude** en commits ni PRs ("no te adjuntes méritos"). El reparto autor/co-autor entre los dos correos es una suposición pendiente de confirmar. |
| Idioma | Código, comentarios y mensajes de commit en inglés; documentación en español. Cabecera SPDX (`AGPL-3.0-or-later`) en archivos nuevos. |
| Votación | ADR-0006 + edad de cuenta y topes (ADR-0007): dedupe por /24 (IPv4) y /64 (IPv6), el voto del creador cuenta como el primero; cuenta ≥ 600 s para reportar/votar; 30 reportes y 200 votos por día. Subredes en BD como HMAC con clave (ADR-0008). |
| Ruteo | **Fail-closed**: una ruta que cruce un cierre confirmado nunca se devuelve como válida. |

Script auxiliar para commitear con esa identidad (hay que recrearlo si el contenedor se reinicia; no está en el repo):

```bash
#!/usr/bin/env bash
# Uso: gc.sh "<subject>" ["<body>"]   (hacer `git add` antes)
set -euo pipefail
subject="$1"; body="${2:-}"
msg="$subject"; [ -n "$body" ] && msg="$msg

$body"
msg="$msg

Co-authored-by: kripiman <119073556+kripiman@users.noreply.github.com>"
GIT_AUTHOR_NAME="gabriel" GIT_AUTHOR_EMAIL="131714613+gabrielpinones@users.noreply.github.com" \
GIT_COMMITTER_NAME="gabriel" GIT_COMMITTER_EMAIL="131714613+gabrielpinones@users.noreply.github.com" \
  git commit -q -m "$msg"
```

---

## 3. Qué se hizo (Ronda 1)

### 3.1 Hallazgos de la auditoría

| ID | Tema | Estado | Dónde |
|---|---|---|---|
| SEC-01 | Secreto JWT público aceptado en producción | **Hecho** | `26e6b31`, `c8c599b`; tests que leen `.env.example` |
| SEC-02 | Rate limit evadible | **Hecho** | `c19ab3e`: clave por red del cliente (IPv4 /32, IPv6 /64), memoria acotada |
| SEC-03 | Votación manipulable, tipo elegido por el cliente | **Hecho** | `1a59b87`, `77298f4`, `5211499`; ADR-0006/0007; tipo derivado de la categoría en el servidor |
| SEC-04 | Identificador de cuenta público y datos en logs | **Hecho** | DTO público sin `creator_account_id`; Caddy sin logs de peticiones (`84d832f`) |
| SEC-05 | Agotamiento de recursos | **Hecho** | `ee653d1`: plazos, tope de concurrencia, cuerpo ≤ 16 KiB, SSE acotado; paginación; purga |
| SEC-06 | Tokens sin expiración ni forma canónica | **Hecho** | `513a47d`: tokens v2 `baze_v2.<payload>.<hmac>`, rotación con `JWT_SECRET_PREVIOUS` |
| SEC-07 | Caddy con listado de directorios | **Hecho** | `23e23f0` |
| SEC-08 | Contenedores sin endurecer; app como superusuario de BD | **Hecho** | `23e23f0`, `bde9fa6` (rol `baze_app` sin DDL), `5181221` |
| SEC-09 | Configuración sin validar; `/source` sin hash real | **Hecho** | `26e6b31`, `5181221` (commit incrustado en la imagen) |
| SEC-10 | Docs de API en producción; build no hermético | **Hecho** | `24e3a79` (swagger-ui vendorizado, reqwest sin TLS) |
| SEC-11 | Validación de entrada y errores | **Hecho** | `f4ecff7` (descripción por caracteres, sin control/bidi), `4d5f31d` (sin detalles internos, `error_id`) |
| SEC-12 | Dependencias Android antiguas | **Parcial** | Hay Dependabot (cargo, actions, gradle, docker). **No** se subieron versiones ni hay `verification-metadata.xml` de Gradle |
| FUN-01 | Núcleo del backend son stubs | **Parcial** | Hecho: PostGIS, cuentas, reportes, votos, corredor, purga (`77298f4`, `97ffe40`, `513a47d`, `8b4b59e`). **Falta**: cliente Valhalla y Photon (Etapas A/B) |
| FUN-02 | CI roto o no determinista | **Hecho** | `799661d`, `f86f11b`, `24f29f5`, `2ac2760`, `c7811cf`, `d9b7e7e`, `95040c9`, `4331c33`, `6c35ef1` |
| FUN-03 | Android no funcional | **Parcial** | Hecho: build verde, `namespace`, convention plugins, R8, `checkPurity`, licencias. **Falta** todo lo funcional (Etapa C, no autorizada) |
| FUN-04 | Contrato OpenAPI y modelos incompatibles | **Parcial** | Hecho: contrato regenerado y verificado en CI, modelos Android alineados y probados (`144dd18`). **Falta** SSE documentado con `resync` (Etapa 0) y cliente Android sobre fixtures dorados (Etapa C) |
| FUN-05 | Pipeline de datos con errores probables | **Pendiente** | Etapa B |
| FUN-06 | Cobertura de pruebas casi nula | **Hecho** | 134 tests sin BD y 196 con PostGIS (última corrida local), regresión e2e contra el binario real |
| FUN-07 | Gobernanza y documentación | **Parcial** | Hecho: ADR-0007/0008, `docs/operations.md`, SECURITY, CONTRIBUTING, ATTRIBUTION, plantillas, CODEOWNERS. **Falta** decisiones del mantenedor (CLA, buzón de seguridad, PVR) |

### 3.2 Evidencia en el CI real

| Workflow | Resultado | Commit |
|---|---|---|
| Backend CI (fmt, clippy `-D warnings`, tests con servicio PostGIS, e2e `--through 10`, cargo-deny, MSRV 1.88, contrato OpenAPI) | Verde | `17c0087` |
| Infrastructure CI (compose, Caddy, pila real, invariantes, `baze_app` sin DDL) | Verde | `2b182b2` |
| Android CI (lint, tests, `checkPurity`, `assembleDebug`, `assembleRelease` con R8, `licenseeAndroidRelease` + `checkLicenses`) | Verde, run 37221895535 | `0bae8d0` |
| Secrets Audit (gitleaks) | Verde | `a7dbcff` |

`FirstVersion` == `origin/FirstVersion` (54 commits sobre `Initial commit`).

### 3.3 Qué contiene hoy el backend

- Workspace Rust (edition 2024, MSRV 1.88, toolchain 1.97.1) con crates `app`, `shared`, `auth`, `hazards`, `routing`, `geocoding`, `realtime`. Los crates de dominio solo dependen de `shared`.
- `hazards`: servicio sobre PostGIS (`HazardStore`, `CorridorHazards`), votación en una transacción con `SELECT … FOR UPDATE`, índice único parcial `(hazard_id, voter_net) WHERE counts`, resolución terminal por `down - up >= umbral`, consultas de corredor con `ST_DWithin(geography)` e índice GiST, polígonos de bloqueo (buffer de 30 m), más de 500 cierres ⇒ `Unavailable` (fail-closed), purga cada 5 min y al arrancar.
- `auth`: cuentas persistidas, tokens v2 con rotación de clave.
- `app`: router Axum, límites (cuerpo, plazo, concurrencia, rate limit acotado), SSE acotado (25 por cliente, 8192 global, 30 min), `/health`, `/source`, `export-openapi`, `migrate`, `healthcheck`.
- Migraciones SQLx verificadas por checksum; el servidor se niega a arrancar si una migración aplicada cambió.

---

## 4. Qué falta

### 4.1 Fase 0 + A + B (autorizada): hecha

Detalle y evidencia en la sección 6. Lo que queda **fuera** de lo verificado, y es del mantenedor:

- **Datos de la región real**: el smoke prueba Mónaco. El tamaño de Chile (memoria de Planetiler, de Valhalla y de Photon, tiempo de construcción) y el índice de búsqueda nacional no se han probado.
- **Fuente de Photon**: el script acepta un volcado JSON o una base ya construida (GraphHopper publica una por país, pero su servidor está bloqueado desde el sandbox, así que ni la URL de Chile ni su checksum están confirmados). Para buscar sobre el mismo extracto que el resto hace falta exportar un volcado desde una importación Nominatim.
- **Elevación de la región**: las teselas se bajan del bucket público de AWS; la atribución de las fuentes de la región elegida debe confirmarse (`ATTRIBUTION.md`).
- **Encender los motores en producción** (`ENGINES_ENABLED=true`) solo después de construir los datos y comprobar con `scripts/e2e/engines_smoke.py` (sección 5.7).

### 4.2 Planificado, NO autorizado (requiere nueva aprobación del mantenedor)

- **Etapa C — Android funcional**: cliente Ktor real (cuentas, reportes, votos, ruta, SSE con reconexión y `resync`), almacenamiento del token con Android Keystore, permisos en tiempo de ejecución y servicio en primer plano correcto, MapLibre ≥ 11.8 con PMTiles (el catálogo fija 11.7.0, que no lo soporta), estilo en tiempo de ejecución, descarga verificada del mapa, selector de categoría, pantalla "Acerca de". Solo verificable en el CI de Android (~6,5 min por corrida).
- **Etapa D — Documentación de cierre**: ADR-0009 (mapas offline y almacenamiento del token), ADR de despliegue de motores y procedencia de datos, notas de MapLibre en ADR-0003. (Lo que dependa de A/B se documenta dentro de B.)

### 4.3 Deuda conocida fuera de las etapas

- SEC-12: versiones de Android sin actualizar y sin verificación de dependencias de Gradle.
- Riesgo V7: el permiso `pull-requests: read` de gitleaks se añadió de forma preventiva; no se ha podido verificar porque no hay PRs.
- La auditoría original dice 11 tests; los reales eran 10 (dato menor).

### 4.4 Decisiones del mantenedor

1. Reparto autor/co-autor de los dos correos (hoy: autor `gabrielpinones`, co-autor `kripiman`).
2. CLA: redactar el texto o mantener la política interina de `CONTRIBUTING.md` (sin PRs de código externos).
3. Crear el buzón `security@baze.cl` y activar Private Vulnerability Reporting.
4. Región del primer despliegue (el script usa Chile) y si GraphHopper publica un índice de Photon para ella.
5. Ancho de banda de teselas: CDN, almacenamiento externo, o aceptar que Caddy sirva `tiles.pmtiles`.
6. Dominio público y fecha de despliegue (la app de release exigirá `baze.apiBaseUrl`).
7. ¿Abrir un Pull Request de `FirstVersion` hacia `main`?

---

## 5. Cómo retomar

### 5.1 Entorno del sandbox

```bash
# El clúster local no arranca solo si el contenedor se reinició:
pg_ctlcluster 16 main start
export DATABASE_URL=postgres://postgres@127.0.0.1:5432/postgres   # trust auth, solo local; usar con bases desechables
```

### 5.2 Compuerta local antes de cada push

```bash
make backend-check        # fmt, clippy -D warnings, tests, export-openapi --check
make backend-db-test      # incluye las pruebas con PostGIS (necesita DATABASE_URL)
(cd backend && cargo deny check bans licenses sources && cargo +1.88.0 check --locked --workspace)
bash scripts/check-toolchain-sync.sh && bash scripts/compose-check.sh && python3 scripts/check-style.py
shellcheck scripts/*.sh scripts/e2e/*.sh data/scripts/*.sh data/scripts/container/*.sh
url="$(bash scripts/e2e/prepare-db.sh "$DATABASE_URL")"
python3 scripts/e2e/audit_regression.py --database-url "$url" --admin-database-url "$DATABASE_URL" --through 11
```

Tras cada push se revisan los workflows (`backend.yml`, `infra.yml`, `android.yml`, `data-smoke.yml`, `secrets.yml`, `security.yml`) con las herramientas MCP de GitHub y se corrige antes de seguir. Una etapa se cierra solo con CI verde.

### 5.3 Qué no se puede verificar desde el sandbox

| Limitación | Cómo se cubre |
|---|---|
| Sin daemon de Docker | `infra.yml` y `data-smoke.yml` en el CI |
| Sin Android SDK ni Google Maven | `android.yml` (iterar contra el CI real) |
| `download1.graphhopper.com` bloqueado | Fixture de Photon en el smoke; el índice de Chile lo confirma el mantenedor |
| Decisiones legales y de política | Mantenedor (4.4) |

Sí se puede consultar desde el sandbox: GitHub (`git ls-remote`, assets de releases), Maven Central, plugins.gradle.org, la distribución de Gradle y las APIs de Docker Hub y `ghcr.io` (tags y digests).

### 5.4 Reglas del repo que no hay que romper (resumen de AGENTS.md)

- Crates de dominio (`auth`, `hazards`, `routing`, `geocoding`, `realtime`) dependen solo de `shared`; el cableado va en `app`.
- Handlers delgados; GeoJSON en HTTP; configuración solo por variables de entorno validadas.
- Sin Redis/Kafka/microservicios; sin imágenes `latest`; sin secretos en el repo; sin trazas GPS ni ubicaciones puntuales (ADR-0005).
- `core/model` y `core/domain` de Android son Kotlin puro (lo verifica `checkPurity`).
- Las migraciones aplicadas nunca se editan: cambios en un archivo nuevo.

### 5.5 Hechos verificados sobre los motores

- `gisops/valhalla:3.4.0` y `ghcr.io/komoot/photon:0.5.0` **no existen**. Valhalla: imagen oficial `ghcr.io/valhalla/valhalla:3.9.0` por digest. Photon: sin imagen oficial; `infra/photon/Dockerfile` descarga el jar 1.3.0 de la release con `ADD --checksum` sobre Temurin 21 fijado por digest.
- Valhalla: `exclude_polygons` son anillos `[lon, lat]`; el límite por defecto del perímetro total es 10 000 m (`max_exclude_polygons_length`), de ahí el tope de 50 polígonos de 30 m; los errores `170/171/440/441/442` son «no hay ruta»; `valhalla_build_elevation` **no falla** si no puede bajar una tesela (por eso `valhalla-build.sh` lo comprueba); el orden correcto es `build` → elevación → `enhance` → extracto tar.
- Photon 1.3.0: `java -jar photon.jar import -import-file <volcado.jsonl> -data-dir <dir>` y `serve -data-dir <dir> -listen-ip 0.0.0.0`; no registra el texto buscado; formato de volcado en `docs/json-dump-format-0.1.0.md` del repo de Photon. `/status` responde `{"status":"Ok",…}`.
- Planetiler 0.10.2 por digest; con `--osm-path` y `--download`; para Mónaco acepta fuentes diminutas (`--water-polygons-url`, `--natural-earth-url`) de sus recursos de prueba.
- MapLibre Native Android: PMTiles llegó en **11.8.0**; el catálogo fija 11.7.0 (Etapa C); última 13.6.1.
- El contrato de maniobras (`instruction`, `distance_meters`, `time_seconds`, `location`) coincide entre backend y Android.

### 5.6 Validar los motores en el sandbox, sin Docker

Se hizo para esta fase y conviene repetirlo ante cualquier cambio en los clientes o los scripts:

```bash
# Valhalla real: la rueda pyvalhalla trae los binarios (Python 3.12; uv ya está instalado)
uv venv --python 3.12 venv && uv pip install --python venv/bin/python pyvalhalla==3.9.0 numpy
# Extractos de prueba desde GitHub raw (Geofabrik está bloqueado): valhalla/test/data/utrecht_netherlands.osm.pbf
#   y planetiler-core/src/test/resources/monaco-latest.osm.pbf (tag v0.10.2)
# valhalla_build_config/elevation/extract son scripts Python dentro de la rueda (valhalla/*.py): envuélvelos en un
#   directorio con `PYTHONPATH=<site-packages> python <script>.py "$@"` y ponlo en el PATH.
# Sin acceso a AWS: una tesela .hgt sintética (3601×3601, int16 big-endian) en elevation_tiles/N43/N43E007.hgt
CUSTOM_FILES=… INPUT_PBF=… bash data/scripts/container/valhalla-build.sh      # el mismo script que corre en Docker
valhalla_service …/valhalla.json 2                                             # en :8002
# Photon real: el jar de la release (java 21 está instalado)
java -jar photon-1.3.0.jar import -import-file data/fixtures/photon-monaco.jsonl -data-dir <dir> -languages es,en
java -jar photon-1.3.0.jar serve -data-dir <dir> -listen-ip 127.0.0.1 -listen-port 2322
# Backend contra ambos (ENGINES_ENABLED=true) y el smoke:
python3 scripts/e2e/engines_smoke.py --backend http://127.0.0.1:18080 --psql "psql <url> -X -q -t -A"
```

Trampas encontradas: `pkill -f` con una cadena que aparece en tu propio comando mata tu shell; `valhalla_service` ignora SIGTERM mientras atiende (usa `kill -9`); el disco del sandbox es una cuota fija (los `target/` de cargo llegan a 23 GB: usa `CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0` y `cargo clean` cuando haga falta).

### 5.7 Poner en marcha los motores en un despliegue

```bash
make data-build                       # con PHOTON_IMPORT_FILE o PHOTON_DUMP_URL + PHOTON_DUMP_SHA256 para el paso 04
docker compose -f infra/compose.yaml --env-file infra/.env --profile engines up -d --build --wait
# En infra/.env: ENGINES_ENABLED=true; reinicia el backend; comprueba con scripts/e2e/engines_smoke.py
```

---

## 6. Registro de avance de la fase actual

| Etapa | Estado | Commits | Evidencia de CI |
|---|---|---|---|
| Handoff (este documento) | Hecho | `8441244` | Secrets Audit verde |
| 0 — resync SSE, AGENTS.md, licensee | **Hecho** | `a5275e2`, `321a2b2`, `d5fdee2`, `46ab936` | Backend CI verde; Android CI verde (run 37225225205; el primer intento murió por `OutOfMemoryError` del demonio de Gradle y se subió el heap a 3 GB) |
| A — clientes Valhalla y Photon | **Hecho** | `688f0dc`, `5ff9f81`, `f984625`, `7d1d24d`, `32a75bf`, `371c3bd`, `abaccc9` | Backend CI, Infrastructure CI, Secrets y Security verdes en `371c3bd` (e2e hasta el paso 11 incluido) |
| B — pipeline de datos y motores | **Hecho** | `07e4729`, `a76d8a0`, `e538c59`, `e4f7fec`, `b6a912c`, `9e220f3`, `181c52d` | Data pipeline smoke verde en `9e220f3` (run 37227299049: teselas + estilo + motores reales detrás del backend); Infrastructure CI y Backend CI verdes |
| C — Android funcional | No autorizada | — | — |
| D — documentación de cierre | No autorizada | — | — |

### Validación local con motores reales (Etapas 0, A y B)

Además de los motores simulados, el cliente se probó contra **Valhalla 3.9.0 real** (rueda `pyvalhalla` con un extracto de Utrecht y una tesela de elevación sintética) y **Photon 1.3.0 real** (el JAR oficial importando `data/fixtures/photon-monaco.jsonl`). Resultados:

- La ruta base, los textos en español, el ascenso/descenso (`elevation_interval`), el desvío alrededor de un cierre confirmado (a ~65 m del cierre) y el rechazo de uno sin confirmar funcionan tal como en los dobles de prueba.
- Un cierre sobre la vía en el origen o en el destino da `404` (fail-closed), y tres cierres seguidos se esquivan con un solo desvío.
- Un punto fuera del extracto da `404` con el código real de Valhalla.
- La búsqueda devuelve los resultados esperados (`name`, `street` + número, `city`, `country`) y trata `limit=1000&…` como texto.
- Hallazgo corregido: el filtro de logs por defecto ocultaba los avisos del crate `routing` (`abaccc9`).
- Etapa B: el script de construcción de Valhalla (el mismo que corre en Docker) se probó sobre Mónaco con una tesela de elevación sintética, y falla (sin dejar `valhalla_tiles.tar`) si la tesela falta o está truncada; el smoke (`engines_smoke.py`) pasó entero contra Valhalla 3.9.0 y Photon 1.3.0 reales, con rodeos a 33–158 m de cada cierre probado.
- En el CI (`data-smoke.yml`, primera ejecución): descarga verificada con el `.md5` de Geofabrik, Planetiler con fuentes de Mónaco y PMTiles v3 válido, manifiesto coherente con el archivo, estilo aceptado por el validador oficial de MapLibre, grafo de Valhalla con la elevación real descargada de AWS, índice de Photon construido con la imagen propia, pila con los motores sanos, rutas con elevación real y rodeos del cierre, motores sin puertos publicados, usuario no root con sistema de archivos de solo lectura, y ni el texto buscado ni las coordenadas en ningún log.
