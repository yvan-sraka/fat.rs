# `fat.rs`

## How to make a FAT image:

```sh
mkfs.fat -F 32 -s 1 -R 2 -f 1 test.img
```

`-F 32` = faire du FAT32 ; `-s 1` = 1 secteur/cluster ; `-R 2` = 2 secteurs réservés (c'est le minimum) ; `-f 1` = une seule FAT (c'est une feature de redondance). 