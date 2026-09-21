// SPDX-FileCopyrightText: 2026 Gabriel Piñones
// SPDX-License-Identifier: AGPL-3.0-or-later

package cl.baze.app.di

import cl.baze.core.domain.FilterActiveHazardsUseCase
import cl.baze.core.domain.HazardProximityUseCase
import cl.baze.core.location.AndroidLocationManagerProvider
import cl.baze.core.location.LocationProvider
import cl.baze.core.network.BazeApiClient
import org.koin.android.ext.koin.androidContext
import org.koin.dsl.module

val appModule = module {
    // Red y cliente API
    single { BazeApiClient() }

    // Proveedor de geolocalización 100% libre basado en LocationManager
    single<LocationProvider> { AndroidLocationManagerProvider(androidContext()) }

    // Casos de uso de dominio (Kotlin puro)
    factory { HazardProximityUseCase(alertRadiusMeters = 50.0) }
    factory { FilterActiveHazardsUseCase() }
}
