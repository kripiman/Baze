<!--
SPDX-FileCopyrightText: 2026 Gabriel Piñones
SPDX-License-Identifier: AGPL-3.0-or-later
-->

# ADR 0007: Identidad Persistente, Regla de Votación y Límites de Abuso

## Estado
Aceptado. Enmienda a [ADR-0002](0002-reroute-only-on-confirmed-blocking.md) y a [ADR-0006](0006-anonymous-accounts-and-rate-limit.md); la persistencia de la deduplicación por red se decide en [ADR-0008](0008-database-roles-migrations-and-vote-networks.md).

## Contexto
La auditoría de seguridad del proyecto (octubre de 2026) encontró que las defensas descritas en ADR-0006 eran correctas en el papel pero frágiles en la práctica:

- El token anónimo no caducaba, no estaba respaldado por ninguna fila en la base de datos (no se podía revocar ni banear una cuenta) y se podía falsificar si el secreto era el de ejemplo.
- El límite de peticiones se podía evadir cambiando el contenido de la cabecera `Authorization`, porque la clave del contador incluía valores que controla el cliente.
- Todo el estado de reportes y votos vivía en memoria: un reinicio borraba los reportes y la memoria de qué redes ya habían votado.
- Tres cuentas anónimas creadas en un minuto desde tres redes bastaban para confirmar un corte de calle falso y desviar a todos los ciclistas.

## Decisión

### 1. Cuentas persistidas y tokens con caducidad
- Cada cuenta es una fila de `accounts` (`id`, `created_at`, `is_active`). El token no es un secreto compartido de larga vida sino una credencial firmada que se valida contra esa fila en **cada** petición: `UPDATE accounts SET is_active = false WHERE id = ...` deja inservible el token de inmediato, sin caché ni lista de revocación.
- Formato v2: `baze_v2.<payload hex>.<mac hex>`, con `payload = id de cuenta (16 B) ‖ emitido (u32) ‖ caduca (u32) ‖ id de clave (u8)` y `mac = HMAC-SHA-256(clave, "baze/anon-token/v2\0" ‖ payload)`. La etiqueta de dominio impide que una firma producida para otro propósito con el mismo secreto valga como token.
- Vigencia `AUTH_TOKEN_TTL_DAYS` (1 a 730, por defecto 180). Solo se acepta la forma canónica (hexadecimal en minúsculas, longitud exacta): mayúsculas, espacios o relleno dan 401.
- Rotación del secreto sin cortar a nadie: `JWT_SECRET_PREVIOUS` verifica tokens firmados con el secreto anterior hasta que caduquen; nunca firma. El identificador de clave se deriva del propio secreto, así que no hay que configurarlo.
- Los tokens del formato anterior (HMAC sobre el UUID desnudo, sin caducidad) se rechazan. No había clientes en producción, así que no hay migración.
- Cada fallo de autenticación devuelve el mismo 401 genérico (salvo "Token expired"): no se distingue cuenta inexistente de desactivada ni firma inválida de truncada.

### 2. Qué identifica a un cliente para los límites
- La clave del limitador es la **red del cliente** (IPv4 `/32`, IPv6 `/64`), nunca un valor de cabecera controlado por quien llama.
- `X-Real-IP` solo se lee si la conexión física viene de un proxy de `TRUSTED_PROXIES`. En Compose, Caddy tiene IP fija (`172.30.0.2`), el rango dinámico de la red interna no la incluye y es el único proxy de confianza; cualquier otro contenedor no puede falsear la IP.
- El límite por cuenta se aplica solo después de verificar el token, de modo que un token inválido no consume ni crea contadores de cuenta.
- Memoria acotada: como máximo 100 000 claves rastreadas; los clientes que no caben comparten un cubo `overflow` estricto en lugar de hacer crecer la tabla. Toda respuesta 429 lleva `Retry-After`.
- Se mantienen los presupuestos de ADR-0006 (registro: 60 por 15 minutos por red; API: `RATE_LIMIT_REQUESTS_PER_MINUTE`; mutaciones: 30 por minuto por cuenta; SSE: 25 por red y 8192 globales con 30 minutos de vida).
- Además del limitador: tope global de peticiones en vuelo (`MAX_CONCURRENT_REQUESTS`), plazo por petición (`REQUEST_TIMEOUT_SECONDS`, 408), cuerpo máximo de 16 KiB (413) y 10 s para recibir las cabeceras.

### 3. Regla de votación
Se conserva la deduplicación por red de ADR-0006 (solo el primer voto de cada `/24` IPv4 o `/64` IPv6 cuenta para el umbral) y se precisa:
- **El reporte del creador es su primer voto** y cuenta. El creador puede cambiar su voto como cualquier otro, pero no sumar un segundo.
- **El tipo lo deriva el servidor de la categoría** (`glass`, `pothole`, `debris` son `warning`; `road_closed`, `construction`, `flood` son `blocking`). Un `hazard_type` enviado por el cliente se ignora (ya no forma parte de la solicitud), y la base de datos rechaza con un `CHECK` cualquier par categoría/tipo incoherente.
- **Umbral** `HAZARD_CONFIRMATION_THRESHOLD`, entre 2 y 50: con 1, el reportante solo confirmaría su propio bloqueo.
- **Retirada por la comunidad**: cuando `votos en contra − votos a favor ≥ umbral` el reporte pasa a `resolved`, sea aviso o bloqueo. Es un estado terminal: sale de los listados y ya no admite votos (404), aunque después se cambien votos.
- Un reporte caducado deja de existir para los votantes (404) y de los listados, aunque todavía no se haya purgado.
- La boleta conserva el `counts` con el que nació: cambiar de IP después no convierte en válido un voto descartado ni al revés.

### 4. Capas adicionales contra cuentas desechables
- **Edad mínima de cuenta** (`ACCOUNT_MIN_AGE_SECONDS`, 600 por defecto, 0 a 86 400): una cuenta más joven recibe 403 al votar o al reportar un bloqueo. Los avisos (`warning`) siguen abiertos a cuentas nuevas porque solo alertan y nunca desvían rutas.
- **Topes diarios por cuenta**: `REPORTS_PER_ACCOUNT_PER_DAY` (30) reportes y `VOTES_PER_ACCOUNT_PER_DAY` (200) primeras boletas en reportes ajenos, en una ventana de 24 horas; el exceso da 429. Cambiar un voto ya emitido, publicar un reporte o votar el propio no gastan cupo.
- Los topes se cuentan sobre las filas almacenadas dentro de la transacción, bajo un `pg_advisory_xact_lock` por cuenta que se toma antes que cualquier bloqueo de fila: peticiones paralelas no pueden sobrepasarlos y el orden de bloqueos evita interbloqueos. Como cuentan filas, los reportes caducados se conservan 24 horas antes de purgarse; un TTL corto no devuelve el cupo antes de tiempo.

## Consecuencias

### Positivas
- Una cuenta abusiva se puede banear con una sentencia SQL y el efecto es inmediato.
- Reiniciar el servidor ya no pierde reportes ni la memoria de qué redes votaron.
- Crear tres cuentas y votar desde tres redes ya no confirma un bloqueo: hay que esperar diez minutos por cuenta, y cada cuenta tiene topes diarios.
- El límite deja de depender de cabeceras controladas por el cliente.

### Límites que se aceptan y se documentan
- **Atacante paciente con varias redes `/24`**: quien tenga tres o más redes independientes y deje envejecer tres cuentas diez minutos todavía puede confirmar un bloqueo. Las defensas lo encarecen (tiempo, redes y topes) pero no lo impiden. La respuesta operativa es detectar y banear cuentas (ver `docs/operations.md`) y, si hace falta, subir el umbral o la edad mínima.
- **No hay prueba de proximidad**: exigir que el votante esté cerca del reporte sería falsificable (la posición la afirma el cliente) y contradice [ADR-0005](0005-no-telemetry-in-mvp.md), que impide recibir ubicaciones puntuales de los usuarios.
- **Redes con NAT de operador (CGNAT)**: varios ciclistas legítimos tras el mismo `/24` cuentan como un solo votante por reporte; se necesita un tercero de otra red para mover el estado.
- **Detección de ráfagas**: no hay detección automática de patrones de voto coordinado; queda para una etapa posterior si el abuso real lo justifica.
