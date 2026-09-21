<!--
SPDX-FileCopyrightText: 2026 Gabriel Piñones
SPDX-License-Identifier: AGPL-3.0-or-later
-->

# ADR 0004: Licenciamiento AGPL-3.0-or-later y Acuerdo de Contribuciones (CLA)

## Estado
Aceptado

## Contexto
Baze es un proyecto impulsado por la comunidad ciclista y busca garantizar que el software permanezca libre, abierto y transparente para siempre, tanto en el cliente como en el backend.
- La licencia **GNU Affero General Public License v3 (AGPL-3.0-or-later)** previene que proveedores de servicios en la nube privaticen el backend y ofrezcan versiones modificadas a través de la red sin liberar el código fuente (cláusula de la sección 13).
- Por otra parte, la aplicación cliente Android es libre y se publicará en tiendas abiertas como F-Droid.
- Sin embargo, se planifica una futura publicación de la app en **iOS (Apple App Store)**. Los términos y condiciones de uso de la App Store imponen restricciones de gestión de derechos digitales (DRM) y limitaciones de instalación que la Free Software Foundation (FSF) considera jurídicamente incompatibles con las condiciones de la GPL/AGPL pura. Si el proyecto incorpora código de terceros bajo AGPL sin un acuerdo de cesión de derechos o licenciamiento dual, el titular del proyecto no tendría la facultad legal de publicar la aplicación en la App Store sin infringir los derechos de autor de dichos colaboradores.

## Decisión
1. Adoptar **GNU AGPL-3.0-or-later** como licencia principal del código del proyecto en backend y cliente móvil.
2. Implementar un **Acuerdo de Licencia de Colaborador (Contributor License Agreement - CLA)** obligatorio para todas las contribuciones externas al repositorio.
3. El CLA otorga al titular del proyecto una licencia amplia, no exclusiva y transferible para relicenciar o distribuir la aplicación en canales que impongan restricciones adicionales incompatibles con la AGPL (como la App Store de Apple), garantizando a su vez que el proyecto principal siempre permanezca bajo licencia libre.
4. El backend expone el endpoint público `GET /source` que retorna la URL del repositorio y el hash del commit desplegado, facilitando el cumplimiento activo de la sección 13 de la AGPL.

## Consecuencias
### Positivas
- Máxima protección del software libre y reciprocidad en el código del servidor y cliente.
- Seguridad jurídica para compilar y distribuir legalmente en iOS (App Store) en el futuro.
- Claridad para colaboradores externos sobre los derechos y uso del código aportado.

### Negativas / Riesgos
- Requerir la firma de un CLA puede introducir una pequeña fricción para colaboradores esporádicos en GitHub.
