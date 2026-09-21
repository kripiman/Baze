// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

package cl.baze.core.location

import cl.baze.core.model.Coordinates
import kotlinx.coroutines.flow.Flow

/**
 * Interfaz agnóstica de ubicación para Baze.
 * Permite intercambiar implementaciones sin contaminar las capas superiores
 * y preserva la compatibilidad con F-Droid al no depender de Google Play Services.
 */
interface LocationProvider {
    fun observeLocation(): Flow<Coordinates>
    suspend fun getLastKnownLocation(): Coordinates?
}
