# Map packs offered for download (ADR 0016): built from OpenStreetMap with
# `nix build .#mapPacks.ID` (nix/maps), published on the project's mirror.
# Size and SHA-256 are those of the built file; the store installs only a
# file whose bytes match. Like every pack, they can also come from a stick.
let
  mirror = file: [ "https://mirror.canevet.eu/files/resilience/packs/maps/${file}" ];
  osm = "OpenStreetMap contributors (Geofabrik extract), built by Resilience OS";
  attribution = "© OpenStreetMap contributors";
in
[
  {
    id = "map-luxembourg";
    title = "Map: Luxembourg";
    description = "Detailed map of Luxembourg: roads, paths, buildings, places, and points of interest you can search.";
    category = "maps";
    language = "lb";
    file = "map-osm-luxembourg_2026-01.mbtiles";
    size = 29315072;
    sha256 = "2c07ad41ce6b01c187d9795b6391a7f770900a568d4038053c6bcf64bd6f7f60";
    urls = mirror "map-osm-luxembourg_2026-01.mbtiles";
    license = "ODbL-1.0";
    source = osm;
    data_date = "2026-01-01";
    coverage = "Luxembourg, whole country (Geofabrik extract boundary). OpenStreetMap data of 1 January 2026: anything built or changed later is missing, and some paths or places may be wrong or absent.";
    inherit attribution;
  }
  {
    id = "map-bretagne";
    title = "Map: Bretagne (France)";
    description = "Detailed map of Bretagne: roads, paths, buildings, places, and points of interest you can search.";
    category = "maps";
    language = "fr";
    file = "map-osm-bretagne_2026-11.mbtiles";
    size = 177532928;
    sha256 = "ca72abfd5f05d4ee2cf24db38c01948dd8f846f37074ca4d6d5e7291149b5a43";
    urls = mirror "map-osm-bretagne_2026-11.mbtiles";
    license = "ODbL-1.0";
    source = osm;
    data_date = "2026-11-18";
    coverage = "Bretagne (France), the whole region: Côtes-d'Armor, Finistère, Ille-et-Vilaine, Morbihan (Geofabrik extract boundary; Loire-Atlantique is not included). OpenStreetMap data of 18 November 2026; some paths or places may be wrong or absent.";
    inherit attribution;
  }
  {
    id = "map-alsace";
    title = "Map: Alsace (France)";
    description = "Detailed map of Alsace: roads, paths, buildings, places, and points of interest you can search.";
    category = "maps";
    language = "fr";
    file = "map-osm-alsace_2026-11.mbtiles";
    size = 58834944;
    sha256 = "d847523bd977f2f48301a8309a64cf2c4610926c1bd193dfe5442f3b8b0244a7";
    urls = mirror "map-osm-alsace_2026-11.mbtiles";
    license = "ODbL-1.0";
    source = osm;
    data_date = "2026-11-21";
    coverage = "Alsace (France): Bas-Rhin, Haut-Rhin (Geofabrik extract boundary). OpenStreetMap data of 21 November 2026; some paths or places may be wrong or absent.";
    inherit attribution;
  }
  {
    id = "map-aquitaine";
    title = "Map: Aquitaine (France)";
    description = "Detailed map of Aquitaine: roads, paths, buildings, places, and points of interest you can search.";
    category = "maps";
    language = "fr";
    file = "map-osm-aquitaine_2026-11.mbtiles";
    size = 180953088;
    sha256 = "7fd0a4e16c4c44982e55606639fd5c5930f326bc35df2a4c5652c74139112393";
    urls = mirror "map-osm-aquitaine_2026-11.mbtiles";
    license = "ODbL-1.0";
    source = osm;
    data_date = "2026-11-21";
    coverage = "Aquitaine (France): Dordogne, Gironde, Landes, Lot-et-Garonne, Pyrénées-Atlantiques (Geofabrik extract boundary). OpenStreetMap data of 21 November 2026; some paths or places may be wrong or absent.";
    inherit attribution;
  }
  {
    id = "map-auvergne";
    title = "Map: Auvergne (France)";
    description = "Detailed map of Auvergne: roads, paths, buildings, places, and points of interest you can search.";
    category = "maps";
    language = "fr";
    file = "map-osm-auvergne_2026-11.mbtiles";
    size = 118460416;
    sha256 = "08a14ac5b694ef45d03f348dc52cfc9df84149698f14cc5e880fe8f33ca08c85";
    urls = mirror "map-osm-auvergne_2026-11.mbtiles";
    license = "ODbL-1.0";
    source = osm;
    data_date = "2026-11-21";
    coverage = "Auvergne (France): Allier, Cantal, Haute-Loire, Puy-de-Dôme (Geofabrik extract boundary). OpenStreetMap data of 21 November 2026; some paths or places may be wrong or absent.";
    inherit attribution;
  }
  {
    id = "map-basse-normandie";
    title = "Map: Basse-Normandie (France)";
    description = "Detailed map of Basse-Normandie: roads, paths, buildings, places, and points of interest you can search.";
    category = "maps";
    language = "fr";
    file = "map-osm-basse-normandie_2026-11.mbtiles";
    size = 88797184;
    sha256 = "89383dae06b5bc19c59e678c2750c3bf38016f1265461fb1d6ad5fbb17b7baf4";
    urls = mirror "map-osm-basse-normandie_2026-11.mbtiles";
    license = "ODbL-1.0";
    source = osm;
    data_date = "2026-11-21";
    coverage = "Basse-Normandie (France): Calvados, Manche, Orne (Geofabrik extract boundary). OpenStreetMap data of 21 November 2026; some paths or places may be wrong or absent.";
    inherit attribution;
  }
  {
    id = "map-bourgogne";
    title = "Map: Bourgogne (France)";
    description = "Detailed map of Bourgogne: roads, paths, buildings, places, and points of interest you can search.";
    category = "maps";
    language = "fr";
    file = "map-osm-bourgogne_2026-11.mbtiles";
    size = 143933440;
    sha256 = "d44bd51e81c0351220664eeee5737e42a4ed5e7fef71a2574b18a7d6fa07ff91";
    urls = mirror "map-osm-bourgogne_2026-11.mbtiles";
    license = "ODbL-1.0";
    source = osm;
    data_date = "2026-11-21";
    coverage = "Bourgogne (France): Côte-d'Or, Nièvre, Saône-et-Loire, Yonne (Geofabrik extract boundary). OpenStreetMap data of 21 November 2026; some paths or places may be wrong or absent.";
    inherit attribution;
  }
  {
    id = "map-centre";
    title = "Map: Centre-Val de Loire (France)";
    description = "Detailed map of Centre-Val de Loire: roads, paths, buildings, places, and points of interest you can search.";
    category = "maps";
    language = "fr";
    file = "map-osm-centre_2026-11.mbtiles";
    size = 152887296;
    sha256 = "c339cbe61d33daa2d9aa8351cbe745800d0001284dc4995e25862da8abe1541a";
    urls = mirror "map-osm-centre_2026-11.mbtiles";
    license = "ODbL-1.0";
    source = osm;
    data_date = "2026-11-21";
    coverage = "Centre-Val de Loire (France): Cher, Eure-et-Loir, Indre, Indre-et-Loire, Loir-et-Cher, Loiret (Geofabrik extract boundary). OpenStreetMap data of 21 November 2026; some paths or places may be wrong or absent.";
    inherit attribution;
  }
  {
    id = "map-champagne-ardenne";
    title = "Map: Champagne-Ardenne (France)";
    description = "Detailed map of Champagne-Ardenne: roads, paths, buildings, places, and points of interest you can search.";
    category = "maps";
    language = "fr";
    file = "map-osm-champagne-ardenne_2026-11.mbtiles";
    size = 76509184;
    sha256 = "ad79e0d0bca792854997cbb1c8a6958e4183c215d71bb8bdcaf2385cbc9120ab";
    urls = mirror "map-osm-champagne-ardenne_2026-11.mbtiles";
    license = "ODbL-1.0";
    source = osm;
    data_date = "2026-11-21";
    coverage = "Champagne-Ardenne (France): Ardennes, Aube, Marne, Haute-Marne (Geofabrik extract boundary). OpenStreetMap data of 21 November 2026; some paths or places may be wrong or absent.";
    inherit attribution;
  }
  {
    id = "map-corse";
    title = "Map: Corse (France)";
    description = "Detailed map of Corse: roads, paths, buildings, places, and points of interest you can search.";
    category = "maps";
    language = "fr";
    file = "map-osm-corse_2026-11.mbtiles";
    size = 27295744;
    sha256 = "afb178a6e6e511ba30ec8cdc1066468baaff94bf4ad9002faf6c67891fe45bd5";
    urls = mirror "map-osm-corse_2026-11.mbtiles";
    license = "ODbL-1.0";
    source = osm;
    data_date = "2026-11-21";
    coverage = "Corse (France): Corse-du-Sud, Haute-Corse (Geofabrik extract boundary). OpenStreetMap data of 21 November 2026; some paths or places may be wrong or absent.";
    inherit attribution;
  }
  {
    id = "map-franche-comte";
    title = "Map: Franche-Comté (France)";
    description = "Detailed map of Franche-Comté: roads, paths, buildings, places, and points of interest you can search.";
    category = "maps";
    language = "fr";
    file = "map-osm-franche-comte_2026-11.mbtiles";
    size = 90521600;
    sha256 = "6984263e5aa7ba6e56bc2a7e6ab44f23750425e0ea4dca165fb11c74438a7830";
    urls = mirror "map-osm-franche-comte_2026-11.mbtiles";
    license = "ODbL-1.0";
    source = osm;
    data_date = "2026-11-21";
    coverage = "Franche-Comté (France): Doubs, Jura, Haute-Saône, Territoire de Belfort (Geofabrik extract boundary). OpenStreetMap data of 21 November 2026; some paths or places may be wrong or absent.";
    inherit attribution;
  }
  {
    id = "map-haute-normandie";
    title = "Map: Haute-Normandie (France)";
    description = "Detailed map of Haute-Normandie: roads, paths, buildings, places, and points of interest you can search.";
    category = "maps";
    language = "fr";
    file = "map-osm-haute-normandie_2026-11.mbtiles";
    size = 55291904;
    sha256 = "9371b844ab7a2efee1860188c42bac42136060e983116220d55a076b6da366f8";
    urls = mirror "map-osm-haute-normandie_2026-11.mbtiles";
    license = "ODbL-1.0";
    source = osm;
    data_date = "2026-11-21";
    coverage = "Haute-Normandie (France): Eure, Seine-Maritime (Geofabrik extract boundary). OpenStreetMap data of 21 November 2026; some paths or places may be wrong or absent.";
    inherit attribution;
  }
  {
    id = "map-ile-de-france";
    title = "Map: Île-de-France (France)";
    description = "Detailed map of Île-de-France: roads, paths, buildings, places, and points of interest you can search.";
    category = "maps";
    language = "fr";
    file = "map-osm-ile-de-france_2026-11.mbtiles";
    size = 133193728;
    sha256 = "6b9c9a7b5c64fba00cadb573bdc9fe60378607e7b7a3a03fe1660a6f32120eb5";
    urls = mirror "map-osm-ile-de-france_2026-11.mbtiles";
    license = "ODbL-1.0";
    source = osm;
    data_date = "2026-11-21";
    coverage = "Île-de-France (France): Paris and the seven départements around it (Geofabrik extract boundary). OpenStreetMap data of 21 November 2026; some paths or places may be wrong or absent.";
    inherit attribution;
  }
  {
    id = "map-languedoc-roussillon";
    title = "Map: Languedoc-Roussillon (France)";
    description = "Detailed map of Languedoc-Roussillon: roads, paths, buildings, places, and points of interest you can search.";
    category = "maps";
    language = "fr";
    file = "map-osm-languedoc-roussillon_2026-11.mbtiles";
    size = 144642048;
    sha256 = "2dce261954427836ce2dbb4680e11603103ca6992ebd91aa8c7dee569522fd74";
    urls = mirror "map-osm-languedoc-roussillon_2026-11.mbtiles";
    license = "ODbL-1.0";
    source = osm;
    data_date = "2026-11-21";
    coverage = "Languedoc-Roussillon (France): Aude, Gard, Hérault, Lozère, Pyrénées-Orientales (Geofabrik extract boundary). OpenStreetMap data of 21 November 2026; some paths or places may be wrong or absent.";
    inherit attribution;
  }
  {
    id = "map-limousin";
    title = "Map: Limousin (France)";
    description = "Detailed map of Limousin: roads, paths, buildings, places, and points of interest you can search.";
    category = "maps";
    language = "fr";
    file = "map-osm-limousin_2026-11.mbtiles";
    size = 80891904;
    sha256 = "133a05d9c5a1550dcb477dce58f3456daf534c10427ec1c0929529e823f2c640";
    urls = mirror "map-osm-limousin_2026-11.mbtiles";
    license = "ODbL-1.0";
    source = osm;
    data_date = "2026-11-21";
    coverage = "Limousin (France): Corrèze, Creuse, Haute-Vienne (Geofabrik extract boundary). OpenStreetMap data of 21 November 2026; some paths or places may be wrong or absent.";
    inherit attribution;
  }
  {
    id = "map-lorraine";
    title = "Map: Lorraine (France)";
    description = "Detailed map of Lorraine: roads, paths, buildings, places, and points of interest you can search.";
    category = "maps";
    language = "fr";
    file = "map-osm-lorraine_2026-11.mbtiles";
    size = 106151936;
    sha256 = "9a6dd95c1c90f8037f94d19818744999b48bda79fce4bb48e5de19ebfb35b086";
    urls = mirror "map-osm-lorraine_2026-11.mbtiles";
    license = "ODbL-1.0";
    source = osm;
    data_date = "2026-11-21";
    coverage = "Lorraine (France): Meurthe-et-Moselle, Meuse, Moselle, Vosges (Geofabrik extract boundary). OpenStreetMap data of 21 November 2026; some paths or places may be wrong or absent.";
    inherit attribution;
  }
  {
    id = "map-midi-pyrenees";
    title = "Map: Midi-Pyrénées (France)";
    description = "Detailed map of Midi-Pyrénées: roads, paths, buildings, places, and points of interest you can search.";
    category = "maps";
    language = "fr";
    file = "map-osm-midi-pyrenees_2026-11.mbtiles";
    size = 228442112;
    sha256 = "999cf105853f8a3b7acf6223f4465b5f091dbbfabe9e4f3ecc0d7223e2d46a35";
    urls = mirror "map-osm-midi-pyrenees_2026-11.mbtiles";
    license = "ODbL-1.0";
    source = osm;
    data_date = "2026-11-21";
    coverage = "Midi-Pyrénées (France): Ariège, Aveyron, Haute-Garonne, Gers, Lot, Hautes-Pyrénées, Tarn, Tarn-et-Garonne (Geofabrik extract boundary). OpenStreetMap data of 21 November 2026; some paths or places may be wrong or absent.";
    inherit attribution;
  }
  {
    id = "map-nord-pas-de-calais";
    title = "Map: Nord-Pas-de-Calais (France)";
    description = "Detailed map of Nord-Pas-de-Calais: roads, paths, buildings, places, and points of interest you can search.";
    category = "maps";
    language = "fr";
    file = "map-osm-nord-pas-de-calais_2026-11.mbtiles";
    size = 83988480;
    sha256 = "fafcfc4df1305c497879131846baa45a38e73d1e85fc9084d14231e8347ea5cc";
    urls = mirror "map-osm-nord-pas-de-calais_2026-11.mbtiles";
    license = "ODbL-1.0";
    source = osm;
    data_date = "2026-11-21";
    coverage = "Nord-Pas-de-Calais (France): Nord, Pas-de-Calais (Geofabrik extract boundary). OpenStreetMap data of 21 November 2026; some paths or places may be wrong or absent.";
    inherit attribution;
  }
  {
    id = "map-pays-de-la-loire";
    title = "Map: Pays de la Loire (France)";
    description = "Detailed map of Pays de la Loire: roads, paths, buildings, places, and points of interest you can search.";
    category = "maps";
    language = "fr";
    file = "map-osm-pays-de-la-loire_2026-11.mbtiles";
    size = 181800960;
    sha256 = "ca2877d2e7a15dfa6bb8d2724b854e941c20f384af2f2ce57e513065351c2a39";
    urls = mirror "map-osm-pays-de-la-loire_2026-11.mbtiles";
    license = "ODbL-1.0";
    source = osm;
    data_date = "2026-11-21";
    coverage = "Pays de la Loire (France): Loire-Atlantique, Maine-et-Loire, Mayenne, Sarthe, Vendée (Geofabrik extract boundary). OpenStreetMap data of 21 November 2026; some paths or places may be wrong or absent.";
    inherit attribution;
  }
  {
    id = "map-picardie";
    title = "Map: Picardie (France)";
    description = "Detailed map of Picardie: roads, paths, buildings, places, and points of interest you can search.";
    category = "maps";
    language = "fr";
    file = "map-osm-picardie_2026-11.mbtiles";
    size = 75436032;
    sha256 = "a02e5f3e31c981bf0d19c596ada777c40027ff0828132cc2289d9d1daf9d3627";
    urls = mirror "map-osm-picardie_2026-11.mbtiles";
    license = "ODbL-1.0";
    source = osm;
    data_date = "2026-11-21";
    coverage = "Picardie (France): Aisne, Oise, Somme (Geofabrik extract boundary). OpenStreetMap data of 21 November 2026; some paths or places may be wrong or absent.";
    inherit attribution;
  }
  {
    id = "map-poitou-charentes";
    title = "Map: Poitou-Charentes (France)";
    description = "Detailed map of Poitou-Charentes: roads, paths, buildings, places, and points of interest you can search.";
    category = "maps";
    language = "fr";
    file = "map-osm-poitou-charentes_2026-11.mbtiles";
    size = 113373184;
    sha256 = "ed9988dbba1c23f57e8c1cbd6555e3ac7cd9101006e411e6a79595c8c10fd987";
    urls = mirror "map-osm-poitou-charentes_2026-11.mbtiles";
    license = "ODbL-1.0";
    source = osm;
    data_date = "2026-11-21";
    coverage = "Poitou-Charentes (France): Charente, Charente-Maritime, Deux-Sèvres, Vienne (Geofabrik extract boundary). OpenStreetMap data of 21 November 2026; some paths or places may be wrong or absent.";
    inherit attribution;
  }
  {
    id = "map-provence-alpes-cote-d-azur";
    title = "Map: Provence-Alpes-Côte d'Azur (France)";
    description = "Detailed map of Provence-Alpes-Côte d'Azur: roads, paths, buildings, places, and points of interest you can search.";
    category = "maps";
    language = "fr";
    file = "map-osm-provence-alpes-cote-d-azur_2026-11.mbtiles";
    size = 174383104;
    sha256 = "0d122446e31cae5b2ecf11987f94bbbbc63129dd93e9ab66e653816dede09561";
    urls = mirror "map-osm-provence-alpes-cote-d-azur_2026-11.mbtiles";
    license = "ODbL-1.0";
    source = osm;
    data_date = "2026-11-21";
    coverage = "Provence-Alpes-Côte d'Azur (France): Alpes-de-Haute-Provence, Hautes-Alpes, Alpes-Maritimes, Bouches-du-Rhône, Var, Vaucluse (Geofabrik extract boundary). OpenStreetMap data of 21 November 2026; some paths or places may be wrong or absent.";
    inherit attribution;
  }
  {
    id = "map-rhone-alpes";
    title = "Map: Rhône-Alpes (France)";
    description = "Detailed map of Rhône-Alpes: roads, paths, buildings, places, and points of interest you can search.";
    category = "maps";
    language = "fr";
    file = "map-osm-rhone-alpes_2026-11.mbtiles";
    size = 304287744;
    sha256 = "23ecda5d51c88928c542bb5c1b910419df2cc384a48901dca5cbefe1b2d94f88";
    urls = mirror "map-osm-rhone-alpes_2026-11.mbtiles";
    license = "ODbL-1.0";
    source = osm;
    data_date = "2026-11-21";
    coverage = "Rhône-Alpes (France): Ain, Ardèche, Drôme, Isère, Loire, Rhône, Savoie, Haute-Savoie (Geofabrik extract boundary). OpenStreetMap data of 21 November 2026; some paths or places may be wrong or absent.";
    inherit attribution;
  }
  {
    id = "map-belgium";
    title = "Map: Belgium";
    description = "Detailed map of Belgium: roads, paths, buildings, places, and points of interest you can search.";
    category = "maps";
    language = "nl";
    file = "map-osm-belgium_2026-11.mbtiles";
    size = 266002432;
    sha256 = "d88ed4e029852567f91580ece33c85f862c70198670b79768762add7de8ad73a";
    urls = mirror "map-osm-belgium_2026-11.mbtiles";
    license = "ODbL-1.0";
    source = osm;
    data_date = "2026-11-21";
    coverage = "Belgium: the whole country (Geofabrik extract boundary). OpenStreetMap data of 21 November 2026; some paths or places may be wrong or absent.";
    inherit attribution;
  }
  {
    id = "map-switzerland";
    title = "Map: Switzerland";
    description = "Detailed map of Switzerland: roads, paths, buildings, places, and points of interest you can search.";
    category = "maps";
    language = "de";
    file = "map-osm-switzerland_2026-11.mbtiles";
    size = 314462208;
    sha256 = "352d2c2c8f6aa24aba5e0cd75ff5a001820b17c59afef8f2f44084c7c14d4c51";
    urls = mirror "map-osm-switzerland_2026-11.mbtiles";
    license = "ODbL-1.0";
    source = osm;
    data_date = "2026-11-21";
    coverage = "Switzerland: the whole country (Geofabrik extract boundary). OpenStreetMap data of 21 November 2026; some paths or places may be wrong or absent.";
    inherit attribution;
  }
]
