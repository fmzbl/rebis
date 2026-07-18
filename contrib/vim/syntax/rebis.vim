if exists("b:current_syntax")
  finish
endif

syntax match rebisOperator /\[\]\|->\|<-/
syntax match rebisAtom /[^][()[:space:]]\+/
syntax match rebisParen /[()]/

highlight default link rebisOperator Operator
highlight default link rebisAtom Identifier
highlight default link rebisParen Delimiter

let b:current_syntax = "rebis"
