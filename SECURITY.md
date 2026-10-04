<!--
SPDX-FileCopyrightText: 2026 Gabriel Piñones
SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Política de Seguridad

La seguridad de nuestros usuarios, de la infraestructura y de los datos de navegación es prioritaria en **Baze**.

## Versiones con soporte

Solo se corrigen vulnerabilidades en la rama principal y en la última versión publicada. El proyecto está en fase inicial: todavía no hay un despliegue público, y hasta que lo haya no existen versiones anteriores que mantener.

## Reporte de Vulnerabilidades

Si descubres una vulnerabilidad de seguridad en cualquier componente (backend, app Android, pipeline o infraestructura), te solicitamos **no abrir un issue público**.

Por favor reporta los detalles mediante nuestro canal privado:
- **Correo de seguridad**: security@baze.cl <!-- TODO(verify): el mantenedor debe confirmar que este buzón existe antes de la primera publicación; si no, retirar esta línea y dejar solo Private Vulnerability Reporting -->
- O mediante la funcionalidad de **Private Vulnerability Reporting** de GitHub en este repositorio (pestaña *Security* > *Report a vulnerability*). <!-- TODO(verify): activar esta opción en la configuración del repositorio -->

Incluye la siguiente información en tu reporte:
1. Tipo y descripción del problema.
2. Pasos detallados para reproducir el fallo o prueba de concepto (PoC).
3. Posible impacto en la privacidad o disponibilidad del servicio.

## Plazos y Proceso de Respuesta

Estos plazos son los que el proyecto se propone cumplir; los mantenedores son voluntarios.

| Etapa | Plazo objetivo |
|---|---|
| Acuse de recibo | 72 horas |
| Triaje (confirmar, clasificar y comunicar la valoración) | 7 días |
| Corrección de una vulnerabilidad crítica o alta | 30 días |
| Divulgación pública coordinada | Con la corrección publicada, o a los 90 días del reporte, lo que ocurra primero |

Mantendremos al reportante informado del avance y, si lo desea, lo acreditaremos al publicar el aviso.

## Alcance

Dentro del alcance: el código de este repositorio, los contenedores y la configuración de `infra/`, y los flujos de CI.

Fuera del alcance: ataques de denegación de servicio volumétricos, ingeniería social, pruebas contra infraestructura de terceros (OpenStreetMap, servicios de teselas ajenos) y hallazgos que dependan de un dispositivo ya comprometido.

Por favor, no accedas a datos de otras personas ni degrades un servicio en producción para demostrar un problema: una prueba de concepto contra tu propia instancia (`make dev-up`) es suficiente.

## Despliegues y secretos

Los pasos para generar, rotar y custodiar los secretos, y para revocar una cuenta abusiva, están en [docs/operations.md](docs/operations.md). Si un despliegue arrancó alguna vez con el valor de ejemplo que traía `infra/.env.example` en versiones anteriores, debe considerarse comprometido: hay que rotar `JWT_SECRET` y las contraseñas de la base de datos.

Agradecemos a la comunidad por cooperar en la divulgación responsable de vulnerabilidades.
