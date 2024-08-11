# Configuration file for the Sphinx documentation builder.
#
# For the full list of built-in configuration values, see the documentation:
# https://www.sphinx-doc.org/en/master/usage/configuration.html

# -- Project information -----------------------------------------------------
# https://www.sphinx-doc.org/en/master/usage/configuration.html#project-information

import sys
import os
project = 'Slint-python'
copyright = '2024, Slint Developers'
author = 'Slint Developers'

# -- General configuration ---------------------------------------------------
# https://www.sphinx-doc.org/en/master/usage/configuration.html#general-configuration

extensions = ['autodoc2', "myst_parser", "sphinx_copybutton"]

autodoc2_packages = [
    {
        "path": "../../slint",
    }
]

autodoc2_hidden_objects = ["private", "dunder"]

autodoc2_docstring_parser_regexes = [
    # this will render all docstrings as Markdown
    (r".*", "myst"),
]

autodoc_typehints = "description"

templates_path = ['_templates']
exclude_patterns = []

sys.path.insert(0, os.path.abspath('../'))


# -- Options for HTML output -------------------------------------------------
# https://www.sphinx-doc.org/en/master/usage/configuration.html#options-for-html-output

# html_theme = 'alabaster'
html_theme = "furo"
html_static_path = ['_static']
