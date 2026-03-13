# 3cat Downloader (o 3xl/sx3)

_(English)_ This is a downloader for TV shows and movies from 3cat.cat, the Catalan national TV channel. For the rest of the README, the instructions will be in Catalan. This script was done in an afternoon to download Dragon Ball for my son 🐉

Aquest es un programa per descarregar series i pel·licules de 3cat.cat (o d'altres plataformes de la televisio catalana com Super3 o 3xl).

## Us

Tens dues opcions per utilitzar aquest programa: pots clonar el repositori i executar-lo manualment (necessitaras tenir Rust instal·lat), o be descarregar directament el binari.

### Executar el programa manualment

Clona aquest repositori i executa el programa amb la seguent comanda:

```bash
cargo run -- bola-de-drac --directory ~/Downloads/bola-de-drac/
```

### Executar el binari descarregat

Descarrega el binari des de [releases](https://github.com/mcamara/3cat_show_downloader/releases) i executa la seguent comanda:

```bash
./cat_show_downloader bola-de-drac --directory ~/Downloads/bola-de-drac/
```

### Opcions

| Opcio | Curt | Descripcio | Per defecte |
|---|---|---|---|
| `<SLUG>` | | Slug de la serie o pel·licula (veure mes avall) | *obligatori* |
| `--directory` | `-d` | Directori on desar els fitxers | *obligatori* |
| `--start-from-episode` | `-s` | Numero de capitol des del qual comencar (ignorat per pel·licules) | `1` |
| `--concurrent-downloads` | `-c` | Numero de fitxers a descarregar alhora (1-10) | `2` |
| `--skip-subtitles` | | No descarregar els subtitols | `false` |
| `--fix-existing-subtitles` | `-f` | Netejar els fitxers de subtitols (.vtt) ja descarregats al directori | `false` |
| `--embed-existing-subtitles` | | Netejar i incrustar els subtitols als videos ja descarregats (requereix ffmpeg) | `false` |

Per exemple, per descarregar una serie amb 4 capitols alhora en paral·lel:

```bash
./cat_show_downloader bola-de-drac -d ~/Downloads/bola-de-drac/ -c 4
```

Per descarregar una pel·licula:

```bash
./cat_show_downloader iron-man -d ~/Downloads/movies/
```

Per descarregar sense subtitols:

```bash
./cat_show_downloader bola-de-drac -d ~/Downloads/bola-de-drac/ --skip-subtitles
```

Per netejar els subtitols ja descarregats (elimina les capcaleres `Region:` no estandard i els atributs `region:rN` de les linies de temps):

```bash
./cat_show_downloader bola-de-drac -d ~/Downloads/bola-de-drac/ -f
```

Per netejar i incrustar els subtitols existents directament als fitxers de video (requereix ffmpeg instal·lat):

```bash
./cat_show_downloader bola-de-drac -d ~/Downloads/bola-de-drac/ --embed-existing-subtitles
```

### Integracio amb ffmpeg

Si tens [ffmpeg](https://ffmpeg.org/) instal·lat i accessible al PATH del sistema, els subtitols s'incrustaran automaticament als fitxers de video durant la descarrega. Els subtitols VTT es converteixen a format ASS (Advanced SubStation Alpha) per preservar l'estil original (colors, fons, etc.) i s'incrusten en un fitxer `.mkv` (Matroska) en lloc de `.mp4`. Els fitxers `.vtt` s'eliminen automaticament un cop incrustats.

Si ffmpeg no esta instal·lat, els subtitols es descarregaran com a fitxers `.vtt` separats i el video es mantindra com a `.mp4` (el comportament original).

L'opcio `--embed-existing-subtitles` permet incrustar els subtitols als videos que ja s'han descarregat previament. Aquesta opcio tambe neteja els subtitols (igual que `--fix-existing-subtitles`) abans d'incrustar-los. Un cop incrustats, els fitxers `.vtt` i `.mp4` originals s'eliminen i es genera un fitxer `.mkv`. Aquesta opcio requereix que ffmpeg estigui instal·lat.

### Com trobar el "slug"?

El "slug" és el nom de la sèrie en minúscules i sense espais. Si tens la URL de la pàgina de la sèrie, el "slug" és la part que apareix després de `/3cat/` i abans del següent `/` (si és de 3cat.cat). En el cas de Súper3 o 3xl, el "slug" apareix després de `/tc3/sx3/` i abans del següent `/`.

Per exemple:

- La URL de la serie "Bola de Drac" es https://www.3cat.cat/3cat/bola-de-drac/, el seu "slug" es `bola-de-drac`.
- Si fos una serie de Super3, com per exemple https://www.3cat.cat/tv3/sx3/kuroko-basquet/, el "slug" seria `kuroko-basquet`.
- Per a una pel·licula, el "slug" es troba de la mateixa manera a la URL de la seva pagina a 3cat.cat.

El programa detecta automaticament si el slug correspon a una serie o a una pel·licula.

### Problemes coneguts (i que probablement no solucionare mai, per ser sincers)

- Despres de descarregar alguns capitols, pot apareixer un problema de xarxa, ja que TV3 "et fa fora". En aquest cas, prova d'executar el programa novament despres d'una estona. El programa no descarregara capitols ja descarregats (i si saps el numero de capitol on ha parat, pots indicar-ho amb l'opcio `--start-from-episode`).
- Si no ets a Catalunya o Andorra, algunes series o pel·licules poden no estar disponibles. Aixo es pot solucionar facilment amb una VPN.

Si tens algun problema, pots crear una issue a [GitHub](https://github.com/mcamara/3cat_show_downloader/issues), i l'analitzare quan tingui temps.

### Coses que potser millori en algun moment

- Testejar/Mockejar l'API de TV3 per a mes fiabilitat.

### Nota personal

Aquest programa el vaig fer en una tarda per poder descarregar Bola de Drac per al meu fill. Espero que et sigui util i si vols ajudar a millorar-ho, no dubtis a crear un PR i enviar les teves millores.
