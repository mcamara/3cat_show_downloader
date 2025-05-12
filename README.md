# 3cat Downloader (or 3xl/sx3)

_(English)_ This is a downloader for TV shows from 3cat.cat, the Catalan national TV channel. For the rest of the README, the instructions will be in Catalan. This script was done in an afternoon to download Dragon Ball for my son 🐉

Aquest és un script per descarregar les sèries de 3cat.cat (o d'altres plataformes de la televisió catalana com Súper3 o 3xl).

## Taula de continguts

- [3cat Downloader (or 3xl/sx3)](#3cat-downloader-or-3xlsx3)
  - [Taula de continguts](#taula-de-continguts)
  - [Prerequisits](#prerequisits)
  - [Ús](#ús)
    - [Executar el script manualment](#executar-el-script-manualment)
    - [Executar el binari descarregat](#executar-el-binari-descarregat)
    - [Veure més missatges d'informació](#veure-més-missatges-dinformació)
  - [Com trobar el "slug" de la sèrie?](#com-trobar-el-slug-de-la-sèrie)
  - [Problemes coneguts (i que probablement no solucionaré mai, per ser sincer)](#problemes-coneguts-i-que-probablement-no-solucionaré-mai-per-ser-sincer)
  - [Coses que potser millori en algun moment](#coses-que-potser-millori-en-algun-moment)
  - [Nota personal](#nota-personal)

## Prerequisits

- [ffmpeg](https://ffmpeg.org/) eina per processar vídeos. Si no el tens instal·lat, pots fer-ho amb la següent comanda:
  - **Ubuntu**:

    ```bash
    sudo apt install ffmpeg
    ```

  - **MacOS**:

    ```bash
    brew install ffmpeg
    ```

  - **Windows**:
  
    ```powershell
    winget install Gyan.FFmpeg
    ```

- [yt-dlp](https://github.com/yt-dlp/yt-dlp) eina per descarregar vídeos d'Internet. Si no el tens instal·lat, pots fer-ho amb la següent comanda:

    ```bash
    pipx install yt-dlp
    ```

## Ús

Tens dues opcions per utilitzar aquest script: pots clonar el repositori i executar el script manualment (necessitaràs tenir Rust instal·lat), o bé descarregar directament el binari.

### Executar el script manualment

Clona aquest repositori i executa el script amb la següent comanda:

```bash
cargo run -- bola-de-drac --directory ~/Downloads/bola-de-drac/
```

### Executar el binari descarregat

Descarrega el binari des de [releases](https://github.com/mcamara/3cat_show_downloader/releases) i executa la següent comanda:

```bash
./cat_show_downloader bola-de-drac --directory ~/Downloads/bola-de-drac/
```

### Veure més missatges d'informació

Si vols veure més missatges d'informació, pots afegir l'opció `--verbose` a la comanda. Això et mostrarà més detalls sobre el procés de descàrrega. La opció `--verbose` es pot especificar múltiples vegades per seguir incrementant el nivell d'informació. Per exemple:

```bash
cargo run -- bola-de-drac --directory ~/Downloads/bola-de-drac/ --verbose
```

## Com trobar el "slug" de la sèrie?

El "slug" és el nom de la sèrie en minúscules i sense espais. Si tens la URL de la pàgina de la sèrie, el "slug" és la part que apareix després de `/3cat/` i abans del següent `/` (si és de 3cat.cat). En el cas de Súper3 o 3xl, el "slug" apareix després de `/tc3/sx3/` i abans del següent `/`.

Per exemple:

- La URL de la sèrie "Bola de Drac" és <https://www.3cat.cat/3cat/bola-de-drac/>, el seu "slug" és `bola-de-drac`.
- Si fos una sèrie de Súper3, com per exemple <https://www.3cat.cat/tv3/sx3/kuroko-basquet/>, el "slug" seria `kuroko-basquet`.

## Problemes coneguts (i que probablement no solucionaré mai, per ser sincer)

- Després de descarregar alguns capítols, pot aparèixer un problema de xarxa, ja que TV3 "et fa fora". En aquest cas, prova d'executar el script novament després d'una estona. El script no descarregarà capítols ja descarregats (i si saps el número de capítol on ha parat, pots indicar-ho amb l'opció `--start-from-episode`).
- Si no estàs a Catalunya o Andorra, algunes sèries poden no estar disponibles. Això es pot solucionar fàcilment amb una VPN.

Si tens algun problema, pots crear una issue a [GitHub](https://github.com/mcamara/3cat_show_downloader/issues), i l'analitzaré quan tingui temps.

## Coses que potser millori en algun moment

- ~~Escollir si descarregar o no els subtítols.~~
- ~~Descarregues en paral·lel per millorar la velocitat.~~
- Testejar/Mockejar l'API de TV3 per a més fiabilitat.
- Millorar la gestió d'errors, com per exemple mostrar missatges més clars en cas de fallada de xarxa.

## Nota personal

Aquest script el vaig fer en una tarda per poder descarregar Bola de Drac per al meu fill. Espero que et sigui útil i si vols ajudar a millorar-ho, no dubtis a crear un PR i enviar les teves millores.
