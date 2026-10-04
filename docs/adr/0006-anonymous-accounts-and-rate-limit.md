<!--
SPDX-FileCopyrightText: 2026 Gabriel Piñones
SPDX-License-Identifier: AGPL-3.0-or-later
-->

# ADR 0006: Cuentas Anónimas, Integridad del Voto Anti-Sybil y Rate Limiting

## Estado
Aceptado. Enmendado por [ADR-0007](0007-identity-voting-and-abuse-limits.md) (identidad persistente, regla de votación, edad de cuenta y topes) y [ADR-0008](0008-database-roles-migrations-and-vote-networks.md) (la deduplicación por red ya no está en memoria).

## Contexto
Para fomentar la participación ciudadana en reportes y alertas viales, la fricción de entrada debe ser mínima: obligar al usuario a registrarse con correo electrónico, contraseñas o cuentas de terceros antes de usar la app desalienta la adopción.

Sin embargo, las cuentas anónimas conllevan dos riesgos de diseño críticos:
1. **Ataque Sybil en Votaciones**: Un atacante automatizado puede emitir múltiples solicitudes de registro anónimo o rotar de red para confirmar bloqueos falsos o desconfirmar incidentes reales, desvirtuando el algoritmo de ruteo ciclista.
2. **Colisión en Redes Móviles (CGNAT)**: En redes celulares (4G/5G), cientos o miles de usuarios comparten una única dirección IPv4 pública provista por el operador. Un rate limit por IP excesivamente estricto en el registro o en la navegación bloquea a usuarios legítimos con errores HTTP 429 durante el onboarding.

El rate limit por IP por sí solo no resuelve el problema Sybil (solo ralentiza la creación de identidades). La defensa anti-abuso debe desacoplar la velocidad de red de la legitimidad del consenso comunitario.

## Decisión

### 1. Emisión de Cuentas Anónimas y Presupuestos de Red
- En el primer inicio, la app móvil solicita un token mediante `POST /api/v1/auth/anonymous`.
- El cliente almacena el token en almacenamiento seguro (Android Keystore / EncryptedSharedPreferences).
- **Dos Políticas Canónicas de Red**:
  - **Política Cliente (`client_network`)**: IPv4 `/32`, IPv6 `/64`.
    - `SIGNUP_BUDGET`: 60 cuentas cada 15 minutos por clave de cliente.
    - `API_BUDGET`: 60 peticiones/minuto por clave de cliente para endpoints públicos.
    - **Concurrencia SSE**: 25 conexiones concurrentes por clave de cliente (`/64` en IPv6) y 8192 globales, con desconexión determinista a los 30 minutos. Garantiza que múltiples ciclistas en antenas celulares 3GPP (cada uno con su propio `/64`) no compartan ni saturen el límite de 25 conexiones mientras navegan.
  - **Política Votante (`VoterNetwork`)**: IPv4 `/24`, IPv6 `/64`.
  - `ACCOUNT_MUTATION_BUDGET`: 30 operaciones/minuto por `account_id` autenticado para creación y votación de reportes.
  - **Función canónica de red**: `shared::network_of(ip, v4_prefix, v6_prefix) -> IpNet` y `shared::client_network(ip) -> IpNet` garantizan tipado estricto con cero alocaciones de heap.

### 2. Integridad del Voto y Mitigación Anti-Sybil
Para evitar que una misma persona o script confirme o cancele reportes mediante múltiples identidades anónimas, el sistema aplica una regla de **Consenso Multi-Red por Subred** basada en el tipo de dominio `VoterNetwork` (IPv4 `/24`, IPv6 `/64`):
1. **Fijación de Cuenta a su Registro de Votación (`Ballot`)**:
   - Cada cuenta que vota por primera vez en un reporte obtiene una boleta (`Ballot { vote, counts }`).
   - El indicador `counts` se define atómicamente al primer voto: `counts = claimed_subnets.insert(network)`.
   - Votos subsiguientes de esa misma cuenta (incluso si cambia de Wi-Fi a datos móviles o VPN) solo mutan su valor `vote`, preservando el estado original de `counts`. Esto impide que una sola cuenta confirme un bloqueo rotando de IP.
2. **Deduplicación por Subred (`VoterNetwork`)**:
   - Cada reporte vial (`Hazard`) mantiene en memoria el conjunto de subredes que ya aportaron un voto contabilizable (`claimed_subnets: HashSet<VoterNetwork>`).
   - El tipo `VoterNetwork` encapsula la política de red de dominio sin requerir que la capa web calcule o manipule strings ni hashes ad-hoc.
   - Solo el **primer voto** proveniente de una subred determinada contabiliza (`counts == true`) hacia el umbral `HAZARD_CONFIRMATION_THRESHOLD` (por defecto 3).
   - Múltiples cuentas en la misma subred no incrementan el balance de confirmación comunitaria. Para confirmar un corte bloqueante (`Confirmed`), el incidente debe recibir respaldo de al menos 3 subredes de red independientes.

## Consecuencias y Límites del Modelo

### Positivas
- Onboarding inmediato sin barreras de entrada ni fricción de registro.
- Resistencia robusta ante ataques Sybil locales: un atacante no puede autofirmar un bloqueo creando múltiples cuentas en su conexión, ni rotando de red con una sola cuenta.
- El aislamiento de suscriptores móviles IPv6 `/64` tanto en rate limits como en SSE evita colisiones indebidas entre usuarios activos pedaleando en redes 3GPP.
- Privacidad por diseño garantizada: no se almacenan trazas de IP en base de datos.

### Limitaciones Conocidas y Alcance Futuro
- **Resuelto**: la deduplicación por subred ya no opera en memoria sino en PostgreSQL, con etiquetas de red con clave (ADR-0008), y la madurez de cuentas se implementó como edad mínima y topes diarios (ADR-0007).
- **Exposición de conexiones SSE ante prefijos IPv6 amplios**: Al asignar SSE a nivel `/64`, un atacante que disponga de una asignación IPv6 estática amplia (como un prefijo `/48` residencial o de Tunnelbroker) podría abrir 25 conexiones concurrentes por cada subred `/64` hasta agotar el límite global del servidor (8192 conexiones). Se asume este riesgo conscientemente como límite documentado para priorizar la usabilidad de usuarios móviles reales en ruta.
- **Colisión legítima en subredes locales**: Si dos ciclistas legítimos están conectados a la misma subred local (ej. la misma red Wi-Fi en IPv4 `/24`) e intentan votar el mismo reporte, solo el primero moverá el contador de confirmación; el segundo verá su boleta registrada pero requerirá que un tercer usuario desde otra red participe para mover el estado.
- **Límite ante atacantes distribuidos**: Un atacante que controle al menos 3 cuentas distintas originadas y votadas desde al menos 3 subredes IPv4 `/24` o IPv6 `/64` independientes aún puede alcanzar el umbral de confirmación.
